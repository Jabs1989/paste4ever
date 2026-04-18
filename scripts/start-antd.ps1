# ==========================================================================
#  start-antd.ps1
# ==========================================================================
#
#  Launches the local `antd` daemon with the env vars Paste4Ever needs to
#  pay for storage on Autonomi (Arbitrum One). Used both by the operator
#  (run it manually) and by watchdog.ps1 (auto-restart on DHT rot).
#
#  First-time setup:
#    1. Copy scripts\start-antd.local.ps1.example  ->  scripts\start-antd.local.ps1
#    2. Fill in your AUTONOMI_WALLET_KEY in the local copy (gitignored).
#    3. Optionally override ANTD_BIN / RUST_LOG there too.
#    4. Then just run:  .\scripts\start-antd.ps1
#
#  The launch is *blocking* -- this script hands control to antd until it
#  exits. The watchdog restarts by killing the antd process and spawning
#  this script again as a child PowerShell window.
# ==========================================================================

$ErrorActionPreference = "Stop"

$localConfig = Join-Path $PSScriptRoot "start-antd.local.ps1"
if (-not (Test-Path $localConfig)) {
    Write-Host "ERROR: Missing $localConfig" -ForegroundColor Red
    Write-Host "       Copy scripts\start-antd.local.ps1.example to scripts\start-antd.local.ps1"
    Write-Host "       and fill in your AUTONOMI_WALLET_KEY before running." -ForegroundColor Yellow
    exit 1
}
. $localConfig

if (-not $env:AUTONOMI_WALLET_KEY) {
    Write-Host "ERROR: AUTONOMI_WALLET_KEY is not set in start-antd.local.ps1" -ForegroundColor Red
    exit 1
}

# Arbitrum One (mainnet) -- these are public and stable across the network.
$env:EVM_RPC_URL               = "https://arbitrum-one.publicnode.com"
$env:EVM_NETWORK               = "arbitrum-one"
$env:EVM_PAYMENT_TOKEN_ADDRESS = "0xa78d8321B20c4Ef90eCd72f2588AA985A4BDb684"
$env:EVM_PAYMENT_VAULT_ADDRESS = "0x9A3EcAc693b699Fc0B2B6A50B5549e50c2320A26"

if (-not $env:RUST_LOG) {
    # Silence the saorsa transport/DHT spam that dominates the log:
    #   - saorsa_transport::high_level::connection = OFF
    #       kills the "os error 10049" IPv6-dual-stack spam on Windows
    #       (saorsa tries ::ffff:x.x.x.x, Windows rejects, it falls back
    #       to IPv4 which works — these errors are cosmetic)
    #   - saorsa_transport = error
    #       keeps any *other* transport-level errors visible
    #   - saorsa_core = error
    #       kills [DUAL SEND] IPv6 warnings + K-bucket-at-capacity
    # Keep antd, ant_core (chunk storage), and evmlib (payments) at info
    # so we still see tx hashes and "chunks stored" lines.
    $env:RUST_LOG = "info,saorsa_core=error,saorsa_transport=error,saorsa_transport::high_level::connection=off,ant_core=info,antd=info,evmlib=info"
}

if (-not $env:ANTD_BIN) {
    $env:ANTD_BIN = "C:\Users\USER\Desktop\Autonomi\ant-sdk\antd\target\release\antd.exe"
}

if (-not (Test-Path $env:ANTD_BIN)) {
    Write-Host "ERROR: antd binary not found at $env:ANTD_BIN" -ForegroundColor Red
    Write-Host "       Build it with:  cd ant-sdk\antd; cargo build --release"
    exit 1
}

Write-Host "Launching antd from $env:ANTD_BIN" -ForegroundColor Cyan
Write-Host "   Wallet:  0x$($env:AUTONOMI_WALLET_KEY.Substring(2,6))..$($env:AUTONOMI_WALLET_KEY.Substring($env:AUTONOMI_WALLET_KEY.Length - 4))"
Write-Host "   Network: $env:EVM_NETWORK"
Write-Host ""

& $env:ANTD_BIN
