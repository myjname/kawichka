param([switch]$Rebuild, [switch]$BuildOnly)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$OutputEncoding = [Console]::OutputEncoding
$launcherRussian = [Globalization.CultureInfo]::CurrentUICulture.TwoLetterISOLanguageName -eq 'ru'
function Launcher-Text([string]$Russian,[string]$English) { if ($launcherRussian) { return $Russian }; return $English }

try {
    $project = Split-Path -Parent $PSScriptRoot
    $application = Join-Path $project 'outputs\Kavychka\Kavychka.exe'
    $stampPath = Join-Path $project 'outputs\launcher-build.json'
    $compiler = Join-Path $env:WINDIR 'Microsoft.NET\Framework64\v4.0.30319\csc.exe'
    if (!(Test-Path -LiteralPath $compiler)) {
        throw (Launcher-Text 'Не найден компилятор .NET Framework. Для сборки нужны Windows x64 и .NET Framework 4.8.' '.NET Framework compiler not found. Building requires Windows x64 and .NET Framework 4.8.')
    }

    $fingerprint = & (Join-Path $PSScriptRoot 'get-source-fingerprint.ps1') -Project $project

    $previous = $null
    if (Test-Path -LiteralPath $stampPath) {
        try { $previous = Get-Content -LiteralPath $stampPath -Raw -Encoding UTF8 | ConvertFrom-Json } catch { }
    }
    $needsBuild = $Rebuild -or !(Test-Path -LiteralPath $application) -or
        $null -eq $previous -or $previous.fingerprint -ne $fingerprint

    if ($needsBuild) {
        Write-Host (Launcher-Text 'Собираю Кавычку. Обычно это занимает несколько секунд...' 'Building Kavychka. This usually takes a few seconds...')
        & (Join-Path $PSScriptRoot 'build.ps1') -NoInstaller
        if ($LASTEXITCODE -ne 0 -or !(Test-Path -LiteralPath $application)) {
            throw (Launcher-Text 'Сборка не завершена. Если Кавычка запущена, закройте её через меню значка в трее и повторите запуск.' 'Build did not complete. If Kavychka is running, exit through its tray menu and try again.')
        }
        @{ fingerprint = $fingerprint } | ConvertTo-Json |
            Set-Content -LiteralPath $stampPath -Encoding UTF8
        Write-Host (Launcher-Text 'Сборка готова.' 'Build ready.')
    } else {
        Write-Host (Launcher-Text 'Приложение уже собрано.' 'The application is already built.')
    }

    if (!$BuildOnly) {
        Write-Host (Launcher-Text 'Запускаю Кавычку...' 'Starting Kavychka...')
        Start-Process -FilePath $application -WorkingDirectory (Split-Path -Parent $application)
    }
    exit 0
} catch {
    Write-Host ''
    Write-Host ((Launcher-Text 'Ошибка: ' 'Error: ') + $_.Exception.Message) -ForegroundColor Red
    exit 1
}
