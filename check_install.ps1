$dirs = @(
    "C:\Program Files\LAV",
    "C:\Program Files (x86)\LAV",
    "$env:LOCALAPPDATA\LAV",
    "$env:APPDATA\LAV",
    "C:\Program Files\lol-account-manager",
    "$env:LOCALAPPDATA\lol-account-manager"
)
foreach ($d in $dirs) {
    if (Test-Path $d) {
        Write-Host "Found: $d"
        Get-ChildItem $d | Select-Object Name, LastWriteTime
    }
}
