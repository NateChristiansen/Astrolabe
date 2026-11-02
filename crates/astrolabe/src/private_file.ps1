# Only language and .NET: a command from a module can fail to load, as when a PowerShell 7
# parent passes its PSModulePath down to this Windows PowerShell.
$ErrorActionPreference = 'Stop'
$me = [Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$wanted = [Security.AccessControl.FileSecurity]::new()
$wanted.SetSecurityDescriptorSddlForm("D:P(A;;FA;;;$me)")
$bytes = [Text.Encoding]::ASCII.GetBytes([Console]::In.ReadToEnd())
$file = [IO.FileStream]::new($env:ASTROLABE_PRIVATE_FILE, [IO.FileMode]::CreateNew,
  [Security.AccessControl.FileSystemRights]::Write, [IO.FileShare]::None, 4096,
  [IO.FileOptions]::None, $wanted)
$file.Write($bytes, 0, $bytes.Length); $file.Close()
$acl = [IO.File]::GetAccessControl($env:ASTROLABE_PRIVATE_FILE)
$all = @($acl.GetAccessRules($true, $true, [Security.Principal.SecurityIdentifier]))
if (-not $acl.AreAccessRulesProtected -or $all.Count -ne 1 -or $all[0].IdentityReference.Value -ne $me) {
  throw "this folder did not keep an owner-only permission: $($acl.GetSecurityDescriptorSddlForm('Access'))" }
