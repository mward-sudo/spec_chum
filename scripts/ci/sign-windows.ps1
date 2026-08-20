# Authenticode-sign Windows binaries when WINDOWS_PFX_BASE64 is set.
# No-ops when the secret is missing so unsigned releases still publish.
$ErrorActionPreference = "Stop"

if (-not $env:WINDOWS_PFX_BASE64) {
    Write-Host "skip: WINDOWS_PFX_BASE64 not set; leaving binaries unsigned"
    exit 0
}
if (-not $env:WINDOWS_PFX_PASSWORD) {
    throw "WINDOWS_PFX_PASSWORD is required when WINDOWS_PFX_BASE64 is set"
}
if ($args.Count -lt 1) {
    throw "usage: sign-windows.ps1 <file> [file...]"
}

$tmp = Join-Path ($env:RUNNER_TEMP -as [string]) "spec-chum-codesign"
New-Item -ItemType Directory -Force -Path $tmp | Out-Null
$pfx = Join-Path $tmp "codesign.pfx"
$thumb = $null

try {
    [IO.File]::WriteAllBytes($pfx, [Convert]::FromBase64String($env:WINDOWS_PFX_BASE64))

    $signtool = Get-ChildItem "C:\Program Files (x86)\Windows Kits\10\bin\*\x64\signtool.exe" |
        Sort-Object FullName -Descending |
        Select-Object -First 1
    if (-not $signtool) {
        throw "signtool.exe not found (Windows 10 SDK)"
    }

    $secure = ConvertTo-SecureString $env:WINDOWS_PFX_PASSWORD -AsPlainText -Force
    $cert = Import-PfxCertificate -FilePath $pfx -CertStoreLocation Cert:\CurrentUser\My -Password $secure
    $thumb = $cert.Thumbprint

    foreach ($file in $args) {
        Write-Host "signtool $file"
        $ok = $false
        for ($attempt = 1; $attempt -le 3; $attempt++) {
            & $signtool.FullName sign /fd SHA256 /td SHA256 `
                /tr "http://timestamp.digicert.com" `
                /sha1 $thumb `
                /d "Spec Chum" `
                $file
            if ($LASTEXITCODE -eq 0) {
                $ok = $true
                break
            }
            Write-Host "signtool attempt $attempt failed (exit $LASTEXITCODE); retrying..."
            Start-Sleep -Seconds 5
        }
        if (-not $ok) { throw "signtool failed for $file after retries" }
    }

    Write-Host "signed $($args -join ' ')"
}
finally {
    if ($null -ne $thumb) {
        Remove-Item "Cert:\CurrentUser\My\$thumb" -ErrorAction SilentlyContinue
    }
    Remove-Item -Force -ErrorAction SilentlyContinue $pfx
    Remove-Item -Recurse -Force -ErrorAction SilentlyContinue $tmp
}
