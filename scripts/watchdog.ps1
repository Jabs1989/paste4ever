# ==========================================================================
#  watchdog.ps1
# ==========================================================================
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
#  You'll see antd's own logs in the window this script opens -- the
#  watchdog just writes a status line every poll.
# ==========================================================================

param(
    [string]$HealthUrl     = "http://localhost:8080/health",
    [int]   $PollSeconds   = 30,
    # 2 consecutive degraded polls = 60s of confirmed DHT rot. Lower than
    # the original 3 because the API now bumps consecutive_failures on
    # every "partial upload" 500 response (not just after all retries
    # exhaust), which means /health flips to degraded faster and a single
    # long-running bad paste is enough signal on its own.
    [int]   $FailThreshold = 2,
    [int]   $BootSeconds   = 45
)

$ErrorActionPreference = "Continue"
$scriptDir   = $PSScriptRoot
$startScript = Join-Path $scriptDir "start-antd.ps1"

if (-not (Test-Path $startScript)) {
    Write-Host "ERROR: Can't find $startScript" -ForegroundColor Red
    exit 1
}

function Get-AntdProcess {
    Get-Process -Name "antd" -ErrorAction SilentlyContinue
}

function Start-Antd {
    Write-Host "[$(Get-Date -Format HH:mm:ss)] SPAWN  Spawning antd via start-antd.ps1..." -ForegroundColor Cyan
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
        Write-Host "[$(Get-Date -Format HH:mm:ss)] KILL   Killing antd PID $($p.Id)" -ForegroundColor Yellow
        try { Stop-Process -Id $p.Id -Force } catch { Write-Host "   failed: $_" -ForegroundColor Red }
    }
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

Write-Host "Paste4Ever watchdog started" -ForegroundColor Green
Write-Host "    url:       $HealthUrl"
Write-Host "    every:     $($PollSeconds)s"
Write-Host "    threshold: $FailThreshold consecutive degraded polls -> restart antd"
Write-Host ""

if (-not (Get-AntdProcess)) {
    Write-Host "[$(Get-Date -Format HH:mm:ss)] antd is not running -- launching now" -ForegroundColor Yellow
    Start-Antd
    Start-Sleep -Seconds $BootSeconds
}

$degradedStreak = 0

while ($true) {
    $h = Get-Health
    $now = Get-Date -Format HH:mm:ss

    if ($null -eq $h) {
        Write-Host "[$now] WARN   API unreachable (is paste4ever-api.exe running?)" -ForegroundColor DarkYellow
        $degradedStreak = 0
    }
    elseif ($h.status -eq "healthy") {
        Write-Host "[$now] OK     healthy  (antd_reachable=$($h.antd_reachable), failures=$($h.consecutive_failures))" -ForegroundColor Green
        $degradedStreak = 0
    }
    else {
        $degradedStreak++
        Write-Host "[$now] DEGR   degraded ($degradedStreak/$FailThreshold) antd_reachable=$($h.antd_reachable), failures=$($h.consecutive_failures)" -ForegroundColor Yellow

        if ($degradedStreak -ge $FailThreshold) {
            Write-Host "[$now] ALARM  threshold hit -- restarting antd" -ForegroundColor Red
            Stop-Antd
            Start-Antd
            Write-Host "[$now] WAIT   waiting $($BootSeconds)s for antd to bootstrap before resuming polls"
            Start-Sleep -Seconds $BootSeconds
            $degradedStreak = 0
        }
    }

    Start-Sleep -Seconds $PollSeconds
}
