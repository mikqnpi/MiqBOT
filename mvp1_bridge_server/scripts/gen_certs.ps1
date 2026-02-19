$ErrorActionPreference = "Stop"
$OutDir = "certs"
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null

$DnsNames = if ($env:MIQBOT_TLS_DNS) { $env:MIQBOT_TLS_DNS } else { "localhost" }
$IpNames = if ($env:MIQBOT_TLS_IPS) { $env:MIQBOT_TLS_IPS } else { "127.0.0.1" }

$serverExt = Join-Path $OutDir "server_san.ext"
$clientExt = Join-Path $OutDir "client.ext"

$altLines = New-Object System.Collections.Generic.List[string]
$altLines.Add("subjectAltName=@alt_names")
$altLines.Add("extendedKeyUsage=serverAuth")
$altLines.Add("[alt_names]")

$dnsIndex = 1
foreach ($dns in $DnsNames.Split(',')) {
  $d = $dns.Trim()
  if ($d.Length -gt 0) {
    $altLines.Add("DNS.$dnsIndex=$d")
    $dnsIndex++
  }
}

$ipIndex = 1
foreach ($ip in $IpNames.Split(',')) {
  $i = $ip.Trim()
  if ($i.Length -gt 0) {
    $altLines.Add("IP.$ipIndex=$i")
    $ipIndex++
  }
}

Set-Content -Path $serverExt -Encoding ASCII -Value $altLines
Set-Content -Path $clientExt -Encoding ASCII -Value "extendedKeyUsage=clientAuth"

openssl genrsa -out "$OutDir\ca.key.pem" 4096
openssl req -x509 -new -nodes -key "$OutDir\ca.key.pem" -sha256 -days 3650 -subj "/CN=MiqBOT-ca" -out "$OutDir\ca.crt.pem"

openssl genrsa -out "$OutDir\server.key.pem" 4096
openssl req -new -key "$OutDir\server.key.pem" -subj "/CN=MiqBOT-bridge" -out "$OutDir\server.csr.pem"
openssl x509 -req -in "$OutDir\server.csr.pem" -CA "$OutDir\ca.crt.pem" -CAkey "$OutDir\ca.key.pem" -CAcreateserial -out "$OutDir\server.crt.pem" -days 3650 -sha256 -extfile "$serverExt"

openssl genrsa -out "$OutDir\client.key.pem" 4096
openssl req -new -key "$OutDir\client.key.pem" -subj "/CN=MiqBOT-game-client" -out "$OutDir\client.csr.pem"
openssl x509 -req -in "$OutDir\client.csr.pem" -CA "$OutDir\ca.crt.pem" -CAkey "$OutDir\ca.key.pem" -CAcreateserial -out "$OutDir\client.crt.pem" -days 3650 -sha256 -extfile "$clientExt"

Write-Host "OK: generated CA/server/client certs into $OutDir\"
Write-Host "DNS SANs: $DnsNames"
Write-Host "IP SANs:  $IpNames"

