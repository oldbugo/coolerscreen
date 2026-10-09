# Elevated: bind Microsoft's in-box WinUSB driver (C:\Windows\INF\winusb.inf) to the screen's
# CDC function, bypassing Device Manager's class filter. Undo: uninstall the device in Device Manager
# and "Scan for hardware changes" (usbser comes back). Logs to work\winusb-install.log
param([string]$Root = (Split-Path -Parent $PSScriptRoot))
$log = Join-Path $Root 'work\winusb-install.log'
function Log($m) { "$(Get-Date -Format 'HH:mm:ss') $m" | Add-Content $log }
Remove-Item $log -ErrorAction SilentlyContinue

Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Drv {
  [StructLayout(LayoutKind.Sequential)] public struct SP_DEVINFO_DATA { public int cbSize; public Guid ClassGuid; public int DevInst; public IntPtr Reserved; }
  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)] public struct SP_DEVINSTALL_PARAMS {
    public int cbSize; public int Flags; public int FlagsEx; public IntPtr hwndParent; public IntPtr InstallMsgHandler;
    public IntPtr InstallMsgHandlerContext; public IntPtr FileQueue; public IntPtr ClassInstallReserved; public int Reserved;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 260)] public string DriverPath; }
  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)] public struct SP_DRVINFO_DATA {
    public int cbSize; public int DriverType; public IntPtr Reserved;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 256)] public string Description;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 256)] public string MfgName;
    [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 256)] public string ProviderName;
    public long DriverDate; public long DriverVersion; }
  [DllImport("setupapi.dll", SetLastError = true)] public static extern IntPtr SetupDiCreateDeviceInfoList(IntPtr g, IntPtr h);
  [DllImport("setupapi.dll", SetLastError = true, CharSet = CharSet.Unicode)] public static extern bool SetupDiOpenDeviceInfo(IntPtr s, string id, IntPtr h, int f, ref SP_DEVINFO_DATA d);
  [DllImport("setupapi.dll", SetLastError = true, CharSet = CharSet.Unicode)] public static extern bool SetupDiSetDeviceRegistryProperty(IntPtr s, ref SP_DEVINFO_DATA d, int p, byte[] b, int n);
  [DllImport("setupapi.dll", SetLastError = true, CharSet = CharSet.Unicode)] public static extern bool SetupDiGetDeviceInstallParams(IntPtr s, ref SP_DEVINFO_DATA d, ref SP_DEVINSTALL_PARAMS p);
  [DllImport("setupapi.dll", SetLastError = true, CharSet = CharSet.Unicode)] public static extern bool SetupDiSetDeviceInstallParams(IntPtr s, ref SP_DEVINFO_DATA d, ref SP_DEVINSTALL_PARAMS p);
  [DllImport("setupapi.dll", SetLastError = true)] public static extern bool SetupDiBuildDriverInfoList(IntPtr s, ref SP_DEVINFO_DATA d, int t);
  [DllImport("setupapi.dll", SetLastError = true, CharSet = CharSet.Unicode)] public static extern bool SetupDiEnumDriverInfo(IntPtr s, ref SP_DEVINFO_DATA d, int t, int i, ref SP_DRVINFO_DATA r);
  [DllImport("newdev.dll", SetLastError = true, CharSet = CharSet.Unicode)] public static extern bool DiInstallDevice(IntPtr hwnd, IntPtr s, ref SP_DEVINFO_DATA d, ref SP_DRVINFO_DATA r, int f, out bool reboot);
  [DllImport("setupapi.dll")] public static extern bool SetupDiDestroyDeviceInfoList(IntPtr s);
}
'@

try {
  $dev = Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -match '^USB\\VID_33C3&PID_77(88|91)&MI_00' } | Select-Object -First 1
  if (-not $dev) { throw 'screen not present' }
  Log "device $($dev.InstanceId) class $($dev.Class) status $($dev.Status)"
  $set = [Drv]::SetupDiCreateDeviceInfoList([IntPtr]::Zero, [IntPtr]::Zero)
  $d = New-Object Drv+SP_DEVINFO_DATA; $d.cbSize = [Runtime.InteropServices.Marshal]::SizeOf($d)
  if (-not [Drv]::SetupDiOpenDeviceInfo($set, $dev.InstanceId, [IntPtr]::Zero, 0, [ref]$d)) { throw "open $([Runtime.InteropServices.Marshal]::GetLastWin32Error())" }

  # Move the device to the USBDevice class so the class-driver list accepts winusb.inf.
  $g = [Text.Encoding]::Unicode.GetBytes('{88BAE032-5A81-49f0-BC3D-A4FF138216D6}' + [char]0)
  if (-not [Drv]::SetupDiSetDeviceRegistryProperty($set, [ref]$d, 8, $g, $g.Length)) { throw "set class $([Runtime.InteropServices.Marshal]::GetLastWin32Error())" }
  [Drv]::SetupDiDestroyDeviceInfoList($set) | Out-Null
  $set = [Drv]::SetupDiCreateDeviceInfoList([IntPtr]::Zero, [IntPtr]::Zero)
  $d = New-Object Drv+SP_DEVINFO_DATA; $d.cbSize = [Runtime.InteropServices.Marshal]::SizeOf($d)
  [Drv]::SetupDiOpenDeviceInfo($set, $dev.InstanceId, [IntPtr]::Zero, 0, [ref]$d) | Out-Null
  Log "class now $($d.ClassGuid)"

  $p = New-Object Drv+SP_DEVINSTALL_PARAMS; $p.cbSize = [Runtime.InteropServices.Marshal]::SizeOf($p)
  [Drv]::SetupDiGetDeviceInstallParams($set, [ref]$d, [ref]$p) | Out-Null
  $p.DriverPath = 'C:\Windows\INF\winusb.inf'
  $p.Flags = $p.Flags -bor 0x10000          # DI_ENUMSINGLEINF
  $p.FlagsEx = $p.FlagsEx -bor 0x800        # DI_FLAGSEX_ALLOWEXCLUDEDDRVS
  if (-not [Drv]::SetupDiSetDeviceInstallParams($set, [ref]$d, [ref]$p)) { throw "params $([Runtime.InteropServices.Marshal]::GetLastWin32Error())" }
  if (-not [Drv]::SetupDiBuildDriverInfoList($set, [ref]$d, 1)) { throw "build $([Runtime.InteropServices.Marshal]::GetLastWin32Error())" }

  $r = New-Object Drv+SP_DRVINFO_DATA; $r.cbSize = [Runtime.InteropServices.Marshal]::SizeOf($r)
  $found = $false
  for ($i = 0; [Drv]::SetupDiEnumDriverInfo($set, [ref]$d, 1, $i, [ref]$r); $i++) {
    Log "driver[$i] $($r.Description) / $($r.MfgName)"
    if ($r.Description -eq 'WinUsb Device') { $found = $true; break }
  }
  if (-not $found) { throw "WinUsb Device not in list (last error $([Runtime.InteropServices.Marshal]::GetLastWin32Error()))" }
  $reboot = $false
  if (-not [Drv]::DiInstallDevice([IntPtr]::Zero, $set, [ref]$d, [ref]$r, 0, [ref]$reboot)) { throw "install $([Runtime.InteropServices.Marshal]::GetLastWin32Error())" }
  Log "installed, reboot needed: $reboot"
  Start-Sleep 2
  $now = Get-PnpDevice -InstanceId $dev.InstanceId
  Log "now: $($now.FriendlyName) class $($now.Class) status $($now.Status)"
} catch { Log "ERROR $_" }
