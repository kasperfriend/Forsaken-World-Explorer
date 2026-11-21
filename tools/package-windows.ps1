param(
    [Parameter(Mandatory = $true)]
    [string]$Executable,

    [Parameter(Mandatory = $true)]
    [string]$Modules,

    [Parameter(Mandatory = $true)]
    [string]$OutputDirectory,

    [string]$ArchivePath = ''
)

$ErrorActionPreference = 'Stop'

$executablePath = (Resolve-Path -LiteralPath $Executable).Path
$modulesPath = (Resolve-Path -LiteralPath $Modules).Path
if (-not (Test-Path -LiteralPath $executablePath -PathType Leaf)) {
    throw "Viewer executable not found: $executablePath"
}
foreach ($requiredManifest in @('Walker/module.ini', 'Populator/module.ini')) {
    if (-not (Test-Path -LiteralPath (Join-Path $modulesPath $requiredManifest) -PathType Leaf)) {
        throw "Required module manifest not found under ${modulesPath}: $requiredManifest"
    }
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$noticesPath = Join-Path $repoRoot 'THIRD_PARTY_NOTICES.md'
if (-not (Test-Path -LiteralPath $noticesPath -PathType Leaf)) {
    throw "THIRD_PARTY_NOTICES.md is missing from the repository root: $noticesPath"
}

$outputPath = [System.IO.Path]::GetFullPath($OutputDirectory)
$packageRoot = Join-Path $outputPath 'Forsaken World Explorer'
if (Test-Path -LiteralPath $packageRoot) {
    Remove-Item -LiteralPath $packageRoot -Recurse -Force
}
New-Item -ItemType Directory -Force -Path $packageRoot | Out-Null

$shippedExecutable = Join-Path $packageRoot 'forsaken-world-explorer.exe'
Copy-Item -LiteralPath $executablePath -Destination $shippedExecutable

$shippedModules = Join-Path $packageRoot 'Modules'
Copy-Item -LiteralPath $modulesPath -Destination $shippedModules -Recurse
Copy-Item -LiteralPath $noticesPath -Destination (Join-Path $packageRoot 'THIRD_PARTY_NOTICES.md')

foreach ($expected in @(
    'forsaken-world-explorer.exe',
    'Modules/Walker/module.ini',
    'Modules/Populator/module.ini',
    'THIRD_PARTY_NOTICES.md'
)) {
    if (-not (Test-Path -LiteralPath (Join-Path $packageRoot $expected) -PathType Leaf)) {
        throw "Staging did not produce expected file: $expected"
    }
}

# Generate SHA256SUMS.txt inside the portable folder covering every shipped file.
$files = @(Get-ChildItem -LiteralPath $packageRoot -Recurse -File |
    Where-Object { $_.FullName -ne (Join-Path $packageRoot 'SHA256SUMS.txt') } |
    Sort-Object FullName)
$checksumLines = @(
    foreach ($file in $files) {
        $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
        $relative = [System.IO.Path]::GetRelativePath($packageRoot, $file.FullName).Replace('\', '/')
        "$hash  $relative"
    }
)
[System.IO.File]::WriteAllLines(
    (Join-Path $packageRoot 'SHA256SUMS.txt'),
    [string[]]$checksumLines,
    [System.Text.Encoding]::ASCII
)

if (-not [string]::IsNullOrWhiteSpace($ArchivePath)) {
    $archiveFullPath = [System.IO.Path]::GetFullPath($ArchivePath)
    $archiveDirectory = Split-Path -Parent $archiveFullPath
    New-Item -ItemType Directory -Force -Path $archiveDirectory | Out-Null
    if (Test-Path -LiteralPath $archiveFullPath) {
        Remove-Item -LiteralPath $archiveFullPath -Force
    }

    Compress-Archive -LiteralPath $packageRoot -DestinationPath $archiveFullPath -CompressionLevel Optimal

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [System.IO.Compression.ZipFile]::OpenRead($archiveFullPath)
    try {
        $entries = @($archive.Entries | ForEach-Object { $_.FullName.Replace('\', '/') } |
            Where-Object { -not $_.EndsWith('/') })
        foreach ($required in @(
            'Forsaken World Explorer/forsaken-world-explorer.exe',
            'Forsaken World Explorer/Modules/Walker/module.ini',
            'Forsaken World Explorer/Modules/Populator/module.ini',
            'Forsaken World Explorer/SHA256SUMS.txt',
            'Forsaken World Explorer/THIRD_PARTY_NOTICES.md'
        )) {
            if ($entries -notcontains $required) {
                throw "Release archive is missing required entry: $required"
            }
        }
        $manifestEntry = $archive.Entries |
            Where-Object { $_.FullName.Replace('\', '/') -eq 'Forsaken World Explorer/SHA256SUMS.txt' } |
            Select-Object -First 1
        $manifestReader = New-Object System.IO.StreamReader($manifestEntry.Open())
        try {
            $manifestLines = @($manifestReader.ReadToEnd() -split "`n" |
                ForEach-Object { $_.Trim() } |
                Where-Object { $_.Length -gt 0 })
        }
        finally {
            $manifestReader.Dispose()
        }
        $manifestPaths = @($manifestLines | ForEach-Object { ($_ -split '\s+', 2)[1] })
        foreach ($entry in $entries) {
            $relative = $entry.Substring('Forsaken World Explorer/'.Length)
            if ($relative -eq 'SHA256SUMS.txt') { continue }
            if ($manifestPaths -notcontains $relative) {
                throw "Release archive entry is not covered by SHA256SUMS.txt: $relative"
            }
        }
        if ($manifestPaths.Count + 1 -ne $entries.Count) {
            throw "SHA256SUMS.txt lists $($manifestPaths.Count) files but the archive contains $($entries.Count) entries including the manifest itself."
        }
    }
    finally {
        $archive.Dispose()
    }

    $archiveHash = (Get-FileHash -LiteralPath $archiveFullPath -Algorithm SHA256).Hash.ToLowerInvariant()
    $archiveName = [System.IO.Path]::GetFileName($archiveFullPath)
    [System.IO.File]::WriteAllLines(
        (Join-Path $archiveDirectory 'SHA256SUMS.txt'),
        [string[]]@("$archiveHash  $archiveName"),
        [System.Text.Encoding]::ASCII
    )
}

Write-Host "Staged portable viewer at $packageRoot"
