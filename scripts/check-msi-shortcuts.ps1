[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$Path
)

$ErrorActionPreference = 'Stop'
$atlasMsiPath = (Resolve-Path -LiteralPath $Path).ProviderPath
$atlasRoot = Split-Path -Parent $PSScriptRoot
$atlasAppId = (Get-Content -LiteralPath (Join-Path $atlasRoot 'src-tauri\tauri.conf.json') -Raw | ConvertFrom-Json).identifier
$atlasInstaller = New-Object -ComObject WindowsInstaller.Installer
$atlasDatabase = $null
$atlasView = $null
$atlasRecord = $null
try {
    $atlasDatabase = $atlasInstaller.OpenDatabase($atlasMsiPath, 0)
    foreach ($atlasShortcutId in @('ApplicationStartMenuShortcut', 'ApplicationDesktopShortcut')) {
        $atlasSql = 'SELECT `Target`, `Icon_` FROM `Shortcut` WHERE `Shortcut` = ''{0}''' -f $atlasShortcutId
        $atlasView = $atlasDatabase.OpenView($atlasSql)
        $atlasView.Execute()
        $atlasRecord = $atlasView.Fetch()
        if (-not $atlasRecord) {
            throw "The MSI is missing $atlasShortcutId."
        }
        if ($atlasRecord.StringData(1) -ne '[!Path]') {
            throw "$atlasShortcutId must target the installed executable."
        }
        if ($atlasRecord.StringData(2)) {
            throw "$atlasShortcutId must use the executable icon, not a per-product MSI icon cache."
        }
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($atlasRecord)
        $atlasRecord = $null
        $atlasView.Close()
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($atlasView)
        $atlasView = $null

        $atlasSql = 'SELECT `PropVariantValue` FROM `MsiShortcutProperty` WHERE `Shortcut_` = ''{0}'' AND `PropertyKey` = ''System.AppUserModel.ID''' -f $atlasShortcutId
        $atlasView = $atlasDatabase.OpenView($atlasSql)
        $atlasView.Execute()
        $atlasRecord = $atlasView.Fetch()
        if (-not $atlasRecord -or $atlasRecord.StringData(1) -ne $atlasAppId) {
            throw "$atlasShortcutId must retain the application's taskbar identity."
        }
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($atlasRecord)
        $atlasRecord = $null
        $atlasView.Close()
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($atlasView)
        $atlasView = $null
    }
    Write-Output 'MSI shortcuts use the installed executable icon and preserve the taskbar identity.'
} finally {
    if ($atlasRecord) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($atlasRecord) }
    if ($atlasView) {
        $atlasView.Close()
        [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($atlasView)
    }
    if ($atlasDatabase) { [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($atlasDatabase) }
    [void][Runtime.InteropServices.Marshal]::FinalReleaseComObject($atlasInstaller)
}
