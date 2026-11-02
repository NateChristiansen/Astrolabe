$ErrorActionPreference = 'Stop'
$me = [Security.Principal.WindowsIdentity]::GetCurrent().User
$wanted = New-Object Security.AccessControl.FileSecurity
$wanted.SetSecurityDescriptorSddlForm("D:P(A;;FA;;;$($me.Value))")
$bytes = [Text.Encoding]::ASCII.GetBytes([Console]::In.ReadToEnd())
$file = New-Object IO.FileStream($env:ASTROLABE_PRIVATE_FILE, [IO.FileMode]::CreateNew,
  [Security.AccessControl.FileSystemRights]::Write, [IO.FileShare]::None, 4096,
  [IO.FileOptions]::None, $wanted)
$file.Write($bytes, 0, $bytes.Length); $file.Close()
$acl = Get-Acl -LiteralPath $env:ASTROLABE_PRIVATE_FILE
$all = @($acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier]))
if (-not $acl.AreAccessRulesProtected -or $all.Count -ne 1 -or $all[0].IdentityReference -ne $me) {
  throw "this folder did not keep an owner-only permission: $($acl.Sddl)" }
