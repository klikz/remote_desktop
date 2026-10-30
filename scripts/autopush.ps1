# O'zgarishlarni avtomatik commit va push qiladi (Windows, PowerShell).
# Ishga tushirish (alohida oynada, ochiq qoldiring):
#   powershell -ExecutionPolicy Bypass -File scripts\autopush.ps1
# To'xtatish: Ctrl+C

param(
    [int]$Interval = 5,   # tekshirish oralig'i, soniya
    [int]$Quiet = 4       # oxirgi o'zgarishdan keyin shuncha soniya kutiladi (fayllar yozib bo'linsin)
)

$ErrorActionPreference = "Continue"
Set-Location (Split-Path -Parent $PSScriptRoot)
Write-Host "Auto-push ishlayapti: $(Get-Location). To'xtatish: Ctrl+C"

$lastState = ""
$lastChange = Get-Date
while ($true) {
    $state = (git status --porcelain) -join "`n"
    if ($state -ne $lastState) {
        $lastState = $state
        $lastChange = Get-Date
    }
    elseif ($state -and ((Get-Date) - $lastChange).TotalSeconds -ge $Quiet) {
        $files = (git status --porcelain | ForEach-Object { $_.Substring(3) }) -join ", "
        if ($files.Length -gt 200) { $files = $files.Substring(0, 200) + "..." }
        $stamp = Get-Date -Format "yyyy-MM-dd HH:mm:ss"
        git add -A
        git commit -q -m "auto: $stamp" -m "O'zgargan: $files"
        git push -q
        if ($LASTEXITCODE -eq 0) {
            Write-Host "[$stamp] push qilindi: $files" -ForegroundColor Green
        } else {
            Write-Host "[$stamp] push xatosi (keyingi urinishda qayta uriniladi)" -ForegroundColor Red
        }
        $lastState = (git status --porcelain) -join "`n"
    }
    elseif (-not $state) {
        # commit qilingan, lekin push bo'lmagan narsa qolgan bo'lsa
        $ahead = git rev-list --count "@{u}..HEAD" 2>$null
        if ($ahead -and [int]$ahead -gt 0) {
            git push -q
            if ($LASTEXITCODE -eq 0) { Write-Host "[$(Get-Date -Format HH:mm:ss)] qolgan commit'lar push qilindi" -ForegroundColor Green }
        }
    }
    Start-Sleep -Seconds $Interval
}
