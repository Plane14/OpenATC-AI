$ErrorActionPreference = 'Stop'
if (!$env:OPENATC_WINDOWS_CERT_PFX_BASE64) { return }
if (!$env:OPENATC_WINDOWS_CERT_PASSWORD) { throw 'A signing certificate password is required' }
$certificatePath = Join-Path $env:RUNNER_TEMP 'openatc-signing.pfx'
try {
    [IO.File]::WriteAllBytes($certificatePath, [Convert]::FromBase64String($env:OPENATC_WINDOWS_CERT_PFX_BASE64))
    $password = ConvertTo-SecureString $env:OPENATC_WINDOWS_CERT_PASSWORD -AsPlainText -Force
    $certificate = Import-PfxCertificate -FilePath $certificatePath -CertStoreLocation Cert:\CurrentUser\My -Password $password -Exportable:$false
    $signer = @($certificate | Where-Object { $_.HasPrivateKey })
    if ($signer.Count -ne 1) { throw 'Expected exactly one signing certificate with a private key' }
    "OPENATC_WINDOWS_SIGN_CERT_SHA1=$($signer[0].Thumbprint)" >> $env:GITHUB_ENV
} finally {
    Remove-Item $certificatePath -Force -ErrorAction SilentlyContinue
}
