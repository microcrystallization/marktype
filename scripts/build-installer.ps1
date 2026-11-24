# Build the x64 Windows installer and portable archive from the locked dependencies.
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root

Write-Host "[1/3] Building the Windows release binary..."
$buildLog = cargo build --release --locked --message-format=json-render-diagnostics
if ($LASTEXITCODE -ne 0) { throw "Cargo release build failed." }

$messages = foreach ($line in $buildLog) { $line | ConvertFrom-Json }
$exe = $messages | Where-Object {
    $_.reason -eq 'compiler-artifact' -and $_.target.name -eq 'marktype' -and $_.executable
} | Select-Object -Last 1 -ExpandProperty executable
$webviewBuild = $messages | Where-Object {
    $_.reason -eq 'build-script-executed' -and $_.package_id -match 'webview2-com-sys'
} | Select-Object -Last 1
if (-not $exe -or -not $webviewBuild) { throw "Cargo did not report the application and its WebView2 loader." }

# A missing PE icon can otherwise slip through compilation and only show up in Explorer.
if (-not ('MarkType.Packaging.ShellIcons' -as [type])) {
    Add-Type -Namespace MarkType.Packaging -Name ShellIcons -MemberDefinition @'
[System.Runtime.InteropServices.DllImport("shell32.dll", EntryPoint = "ExtractIconExW",
    CharSet = System.Runtime.InteropServices.CharSet.Unicode, ExactSpelling = true)]
public static extern uint Count(string path, int index, System.IntPtr large, System.IntPtr small, uint count);
'@
}
$iconCount = [MarkType.Packaging.ShellIcons]::Count($exe, -1, [IntPtr]::Zero, [IntPtr]::Zero, 0)
if ($iconCount -eq 0 -or $iconCount -eq [uint32]::MaxValue) {
    throw "The Windows executable has no embedded application icon: $exe"
}

$loader = Join-Path $webviewBuild.out_dir 'x64\WebView2Loader.dll'
if (-not (Test-Path -LiteralPath $loader -PathType Leaf)) { throw "WebView2Loader.dll was not found at $loader." }
$profile = Split-Path -Parent $exe
Copy-Item -LiteralPath $loader -Destination $profile -Force

$metadata = cargo metadata --no-deps --locked --format-version 1 | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw "Cannot read the package version." }
$version = ($metadata.packages | Where-Object name -eq 'marktype').version
if (-not $version) { throw "The MarkType package version is missing." }
if ($env:GITHUB_REF_TYPE -eq 'tag' -and $env:GITHUB_REF_NAME -ne "v$version") {
    throw "Release tag $env:GITHUB_REF_NAME does not match package version $version."
}

Write-Host "[2/3] Locating the Inno Setup compiler..."
$isccCandidates = @(
    (Get-Command iscc -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source -ErrorAction SilentlyContinue),
    "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe",
    "$env:LOCALAPPDATA\Programs\Inno Setup 7\ISCC.exe",
    "C:\Program Files (x86)\Inno Setup 6\ISCC.exe",
    "C:\Program Files\Inno Setup 6\ISCC.exe",
    "C:\Program Files\Inno Setup 7\ISCC.exe"
)
$iscc = $isccCandidates | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1
if (-not $iscc) { throw "ISCC.exe not found. Install Inno Setup 6 or 7." }

New-Item -ItemType Directory -Force -Path "$root\dist" | Out-Null
Write-Host "[3/3] Compiling the installer and portable archive..."
& $iscc "/DBuildDir=$profile" "/DMyAppVersion=$version" "$root\packaging\windows\marktype.iss"
if ($LASTEXITCODE -ne 0) { throw "Inno Setup compilation failed." }

$zipPath = "$root\dist\MarkType-$version-windows-x64-portable.zip"
Compress-Archive -LiteralPath @(
    $exe,
    (Join-Path $profile 'WebView2Loader.dll'),
    "$root\LICENSE",
    "$root\NOTICE.md",
    "$root\examples"
) -DestinationPath $zipPath -Force

$installer = "$root\dist\MarkType-Setup-$version-x64.exe"
$checksums = foreach ($file in Get-Item -LiteralPath $installer,$zipPath) {
    $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    "$hash  $($file.Name)"
}
$checksums | Set-Content -LiteralPath "$root\dist\SHA256SUMS.txt" -Encoding ascii
Get-Item -LiteralPath $installer,$zipPath,"$root\dist\SHA256SUMS.txt" | Select-Object Name,Length
