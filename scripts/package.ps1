[CmdletBinding()]
param(
    [string]$Target = "x86_64-pc-windows-msvc"
)

$ErrorActionPreference = "Stop"
$RepoRoot = Split-Path -Parent $PSScriptRoot
$Version = (Select-String -LiteralPath (Join-Path $RepoRoot "Cargo.toml") -Pattern '^version = "([^"]+)"$').Matches.Groups[1].Value
$Stage = Join-Path $RepoRoot "target/package/contextpack"
$Dist = Join-Path $RepoRoot "dist"
$Archive = Join-Path $Dist "contextpack-v$Version-windows-x86_64.zip"
$CargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
if ($CargoCommand) {
    $Cargo = $CargoCommand.Source
} else {
    $Cargo = Join-Path $env:USERPROFILE ".cargo/bin/cargo.exe"
}
if (-not (Test-Path -LiteralPath $Cargo)) {
    throw "cargo was not found; install the pinned Rust toolchain first"
}

& $Cargo build --release --locked --target $Target --manifest-path (Join-Path $RepoRoot "Cargo.toml")

if (Test-Path -LiteralPath $Stage) {
    Remove-Item -LiteralPath $Stage -Recurse -Force
}
New-Item -ItemType Directory -Path $Stage -Force | Out-Null
New-Item -ItemType Directory -Path $Dist -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $RepoRoot "target/$Target/release/contextpack.exe") -Destination $Stage
Copy-Item -LiteralPath (Join-Path $RepoRoot "README.md") -Destination $Stage
Copy-Item -LiteralPath (Join-Path $RepoRoot "COMMANDS.md") -Destination $Stage
Copy-Item -LiteralPath (Join-Path $RepoRoot "examples/context-plan.yaml") -Destination $Stage
if (Test-Path -LiteralPath $Archive) {
    Remove-Item -LiteralPath $Archive -Force
}
Compress-Archive -Path (Join-Path $Stage "*") -DestinationPath $Archive
Write-Output $Archive
