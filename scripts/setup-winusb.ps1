# One-time setup (run elevated; CoolerScreen launches it for you via a UAC prompt).
# Binds Microsoft's in-box WinUSB driver (C:\Windows\INF\winusb.inf) to the screen's CDC function
# (USB\VID_33C3&PID_77xx&MI_00) so Windows never sends the serial "DTR low" request that makes the
# firmware turn the display off after 30 s. Safe to run repeatedly. Undo: scripts\restore-serial.ps1
# Log: %TEMP%\coolerscreen-setup.log (last line is "RESULT: OK" or "RESULT: ERROR ...")
$log = Join-Path $env:TEMP 'coolerscreen-setup.log'
function Log($m) { "$(Get-Date -Format 'HH:mm:ss') $m" | Add-Content $log }
Remove-Item $log -ErrorAction SilentlyContinue
$GUID = '{6E45736A-2B1B-4078-B772-B3AF2B6FDE1C}'

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

function Find-Screen { Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -match '^USB\\VID_33C3&PID_77(88|91)&MI_00' } | Select-Object -First 1 }
function Get-Service-Of($id) { try { (Get-PnpDeviceProperty -InstanceId $id -KeyName 'DEVPKEY_Device_Service' -ErrorAction Stop).Data } catch { '' } }

try {
  $dev = Find-Screen
  if (-not $dev) { throw 'screen not present (is it plugged in and powered?)' }
  $id = $dev.InstanceId
  Log "device $id class $($dev.Class) status $($dev.Status) service $(Get-Service-Of $id)"

  if ((Get-Service-Of $id) -eq 'WINUSB') {
    Log 'WinUSB already bound, skipping driver install'
  } else {
    $set = [Drv]::SetupDiCreateDeviceInfoList([IntPtr]::Zero, [IntPtr]::Zero)
    $d = New-Object Drv+SP_DEVINFO_DATA; $d.cbSize = [Runtime.InteropServices.Marshal]::SizeOf($d)
    if (-not [Drv]::SetupDiOpenDeviceInfo($set, $id, [IntPtr]::Zero, 0, [ref]$d)) { throw "open $([Runtime.InteropServices.Marshal]::GetLastWin32Error())" }

    # Move the device to the USBDevice class so the class-driver list accepts winusb.inf.
    $g = [Text.Encoding]::Unicode.GetBytes('{88BAE032-5A81-49f0-BC3D-A4FF138216D6}' + [char]0)
    if (-not [Drv]::SetupDiSetDeviceRegistryProperty($set, [ref]$d, 8, $g, $g.Length)) { throw "set class $([Runtime.InteropServices.Marshal]::GetLastWin32Error())" }
    [Drv]::SetupDiDestroyDeviceInfoList($set) | Out-Null
    $set = [Drv]::SetupDiCreateDeviceInfoList([IntPtr]::Zero, [IntPtr]::Zero)
    $d = New-Object Drv+SP_DEVINFO_DATA; $d.cbSize = [Runtime.InteropServices.Marshal]::SizeOf($d)
    [Drv]::SetupDiOpenDeviceInfo($set, $id, [IntPtr]::Zero, 0, [ref]$d) | Out-Null
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
    if (-not $found) { throw "WinUsb Device not in driver list (last error $([Runtime.InteropServices.Marshal]::GetLastWin32Error()))" }
    $reboot = $false
    if (-not [Drv]::DiInstallDevice([IntPtr]::Zero, $set, [ref]$d, [ref]$r, 0, [ref]$reboot)) { throw "install $([Runtime.InteropServices.Marshal]::GetLastWin32Error())" }
    [Drv]::SetupDiDestroyDeviceInfoList($set) | Out-Null
    Log "installed, reboot needed: $reboot"
    Start-Sleep 2
  }

  # Interface GUID so user-mode apps (libusb) can open the device.
  $dev = Find-Screen
  if (-not $dev) { throw 'screen disappeared after driver install' }
  $id = $dev.InstanceId
  $key = "HKLM:\SYSTEM\CurrentControlSet\Enum\$id\Device Parameters"
  $cur = (Get-ItemProperty -Path $key -Name DeviceInterfaceGUIDs -ErrorAction SilentlyContinue).DeviceInterfaceGUIDs
  if ($cur -notcontains $GUID) {
    New-ItemProperty -Path $key -Name DeviceInterfaceGUIDs -PropertyType MultiString -Value @($GUID) -Force | Out-Null
    Log "set DeviceInterfaceGUIDs on $key"
  } else { Log 'DeviceInterfaceGUIDs already set' }
  Log (pnputil /restart-device "$id" | Out-String).Trim()
  Start-Sleep 2
  $now = Find-Screen
  Log "now: $($now.FriendlyName) class $($now.Class) status $($now.Status) service $(Get-Service-Of $now.InstanceId)"
  Log 'RESULT: OK'
} catch { Log "RESULT: ERROR $_" }
