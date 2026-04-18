# ──────────────────────────────────────────────────────────────────────────
#  watchdog.ps1
# ──────────────────────────────────────────────────────────────────────────
#
#  Polls the Paste4Ever API's /health endpoint every 30s. If the status
#  reports `degraded` for N consecutive polls, the watchdog kills the
#  current antd process and spawns a fresh one via start-antd.ps1.
#
#  Why we need this:
#  antd's peer routing table fills up with stale peers on the early-days
#  Autonomi network (K-bucket capacity 20/20 with no stale peers eligible
#  for eviction). After ~1hr of uptime, uploads stop landing. A restart
#  clears the table and fixes it. Until antd itself gets a smarter
#  eviction policy, this script is our reliability patch.
#
#  Run it in a dedicated PowerShell window:
#    .\scripts\watchdog.ps1
#
#  You'll see antd's own logs in the window this script opens — the
#  watchdog just writes a status line every poll.
# ──────────────────────────────────────────────────────────────────────────

param(
    [string]$HealthUrl     = "http://localhost:8080/health",
    [int]   $PollSeconds   = 30,
    [int]   $FailThreshold = 3,   # consecutive degraded polls before restart
    [int]   $BootSeconds   = 45   # wait after spawn before polling resumes
)

$ErrorActionPreference = "Continue"
$scriptDir   = $PSScriptRoot
$startScript = Join-Path $scriptDir "start-antd.ps1"

if (-not (Test-Path $startScript)) {
    Write-Host "❌ Can't find $startScript" -ForegroundColor Red
    exit 1
}

function Get-AntdProcess {
    # antd.exe — match on image name, not window title, because the watchdog
    # spawns antd in a child window that may have a different title.
    Get-Process -Name "antd" -ErrorAction SilentlyContinue
}

function Start-Antd {
    Write-Host "[$(Get-Date -Format HH:mm:ss)] 🚀 Spawning antd via start-antd.ps1..." -ForegroundColor Cyan
    # Spawn in a new window so antd logs are visible to the operator while
    # this watchdog window keeps showing the poll status.
    Start-Process powershell.exe -ArgumentList @(
        "-NoExit",
        "-ExecutionPolicy", "Bypass",
        "-File", $startScript
    ) | Out-Null
}

function Stop-Antd {
    $procs = Get-AntdProcess
    if (-not $procs) {
        Write-Host "[$(Get-Date -Format HH:mm:ss)] (no antd process to stop)" -ForegroundColor DarkGray
        return
    }
    foreach ($p in $procs) {
        Write-Host "[$(Get-Date -Format HH:mm:ss)] 🛑 Killing antd PID $($p.Id)" -ForegroundColor Yellow
        try { Stop-Process -Id $p.Id -Force } catch { Write-Host "   failed: $_" -ForegroundColor Red }
    }
    # Give the OS a second to release the socket before the next spawn.
    Start-Sleep -Seconds 2
}

function Get-Health {
    try {
        $r = Invoke-RestMethod -Uri $HealthUrl -TimeoutSec 5 -ErrorAction Stop
        return $r
    } catch {
        return $null
    }
}

Write-Host "👁  Paste4Ever watchdog started" -ForegroundColor Green
Write-Host "    url:       $HealthUrl"
Write-Host "    every:     ${PollSeconds}s"
Write-Host "    threshold: $FailThreshold consecutive degraded polls → restart antd"
Write-Host ""

# If antd isn't running at all when we start, launch it immediately.
if (-not (Get-AntdProcess)) {
    Write-Host "[$(Get-Date -Format HH:mm:ss)] antd is not running — launching now" -ForegroundColor Yellow
    Start-Antd
    Start-Sleep -Seconds $BootSeconds
}

$degradedStreak = 0

while ($true) {
    $h = Get-Health
    $now = Get-Date -Format HH:mm:ss

    if ($null -eq $h) {
        # /health itself didn't respond — Rust API is probably down. Not our
        # problem to fix; just log and keep polling.
        Write-Host "[$now] ⚠  API unreachable (is paste4ever-api.exe running?)" -ForegroundColor DarkYellow
        $degradedStreak = 0  # can't judge antd if we can't reach the API
    }
    elseif ($h.status -eq "healthy") {
        Write-Host "[$now] 🟢 healthy  (antd_reachable=$($h.antd_reachable), failures=$($h.consecutive_failures))" -ForegroundColor Green
        $degradedStreak = 0
    }
    else {
        $degradedStreak++
        Write-Host "[$now] 🟡 degraded ($degradedStreak/$FailThreshold) antd_reachable=$($h.antd_reachable), failures=$($h.consecutive_failures)" -ForegroundColor Yellow

        if ($degradedStreak -ge $FailThreshold) {
            Write-Host "[$now] 🚨 threshold hit — restarting antd" -ForegroundColor Red
            Stop-Antd
            Start-Antd
            Write-Host "[$now] ⏳ waiting ${BootSeconds}s for antd to bootstrap before resuming polls"
            Start-Sleep -Seconds $BootSeconds
            $degradedStreak = 0
        }
    }

    Start-Sleep -Seconds $PollSeconds
}
