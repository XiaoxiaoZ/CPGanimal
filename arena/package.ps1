<#
Build CPG Arena and put together a folder (and a zip) to hand out to students.
They need no Rust: only the programs, the Python scripts and Python 3.

    powershell -ExecutionPolicy Bypass -File package.ps1

Creates dist/CPG-Arena with
    arena-gui.exe, arena.exe   the programs
    python/                    arena.py and the GA scripts
    creatures/                 the example creatures (only files tracked by git)
    README.md                  for students (STUDENTS.md)
    GUIDE.md                   the full guide (README.md)
and zips it to dist/CPG-Arena.zip.
#>
Set-Location $PSScriptRoot

# Not under $ErrorActionPreference = "Stop": cargo writes its progress to stderr.
cargo build --release
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
$ErrorActionPreference = "Stop"

$target = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $PSScriptRoot "target" }
$bin = Join-Path $target "release"
$dist = Join-Path $PSScriptRoot "dist"
$out = Join-Path $dist "CPG-Arena"
$zip = Join-Path $dist "CPG-Arena.zip"

if (Test-Path $out) { Remove-Item -Recurse -Force $out }
New-Item -ItemType Directory -Force (Join-Path $out "python"), (Join-Path $out "creatures") | Out-Null

Copy-Item (Join-Path $bin "arena-gui.exe"), (Join-Path $bin "arena.exe") $out
Copy-Item (Join-Path $PSScriptRoot "python\*.py") (Join-Path $out "python")
# Only the examples in git, not creatures trained or designed on this computer.
foreach ($file in git ls-files creatures) {
    Copy-Item (Join-Path $PSScriptRoot $file) (Join-Path $out "creatures")
}
Copy-Item (Join-Path $PSScriptRoot "STUDENTS.md") (Join-Path $out "README.md")
Copy-Item (Join-Path $PSScriptRoot "README.md") (Join-Path $out "GUIDE.md")

if (Test-Path $zip) { Remove-Item -Force $zip }
Compress-Archive -Path $out -DestinationPath $zip

Write-Output ""
Write-Output "Folder: $out"
Write-Output ("Zip:    {0} ({1:N1} MB)" -f $zip, ((Get-Item $zip).Length / 1MB))
