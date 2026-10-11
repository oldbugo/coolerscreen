# Elevated (CoolerScreen launches it via a UAC prompt): reconnect a screen that Windows failed to enumerate at boot
# ("Unknown USB Device (Device Descriptor Request Failed)", instance USB\VID_0000&PID_0002\...).
# It power-cycles the single USB hub port the broken device sits on (IOCTL_USB_HUB_CYCLE_PORT), which makes the
# screen enumerate normally. -DryRun only lists what it would cycle (no admin rights needed). Prefers the port where the screen was seen before; otherwise every failed port.
# Log: -Log <file> (CoolerScreen 2 passes a per-run file; default %TEMP%\coolerscreen-setup.log) (last line is "RESULT: OK" or "RESULT: ERROR ...")
param([string]$Log = (Join-Path $env:TEMP 'coolerscreen-setup.log'), [switch]$DryRun)
$log = $Log
function Log($m) { "$(Get-Date -Format 'HH:mm:ss') $m" | Add-Content $log }
Remove-Item $log -ErrorAction SilentlyContinue
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
using Microsoft.Win32.SafeHandles;
public static class Hub {
  [StructLayout(LayoutKind.Sequential)] public struct SP_DEVICE_INTERFACE_DATA { public int cbSize; public Guid g; public int Flags; public IntPtr Reserved; }
  [StructLayout(LayoutKind.Sequential)] public struct SP_DEVINFO_DATA { public int cbSize; public Guid ClassGuid; public int DevInst; public IntPtr Reserved; }
  [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)] public static extern IntPtr SetupDiGetClassDevs(ref Guid g, string enumerator, IntPtr hwnd, int flags);
  [DllImport("setupapi.dll", SetLastError = true)] public static extern bool SetupDiEnumDeviceInterfaces(IntPtr s, IntPtr d, ref Guid g, int i, ref SP_DEVICE_INTERFACE_DATA r);
  [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)] public static extern bool SetupDiGetDeviceInterfaceDetail(IntPtr s, ref SP_DEVICE_INTERFACE_DATA i, IntPtr detail, int size, out int req, ref SP_DEVINFO_DATA d);
  [DllImport("setupapi.dll", CharSet = CharSet.Unicode, SetLastError = true)] public static extern bool SetupDiGetDeviceInstanceId(IntPtr s, ref SP_DEVINFO_DATA d, System.Text.StringBuilder id, int size, out int req);
  [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)] public static extern SafeFileHandle CreateFile(string n, uint access, uint share, IntPtr sa, uint disp, uint flags, IntPtr t);
  [DllImport("kernel32.dll", SetLastError = true)] public static extern bool DeviceIoControl(SafeFileHandle h, uint code, byte[] inb, int inl, byte[] outb, int outl, out int ret, IntPtr ov);

  public static string PathFor(string instanceId) {
    Guid g = new Guid("f18a0e88-c30c-11d0-8815-00a0c906bed8");
    IntPtr s = SetupDiGetClassDevs(ref g, null, IntPtr.Zero, 0x12); // PRESENT | DEVICEINTERFACE
    for (int i = 0; ; i++) {
      var ifd = new SP_DEVICE_INTERFACE_DATA(); ifd.cbSize = Marshal.SizeOf(ifd);
      if (!SetupDiEnumDeviceInterfaces(s, IntPtr.Zero, ref g, i, ref ifd)) return null;
      var dd = new SP_DEVINFO_DATA(); dd.cbSize = Marshal.SizeOf(dd);
      int req; SetupDiGetDeviceInterfaceDetail(s, ref ifd, IntPtr.Zero, 0, out req, ref dd);
      IntPtr buf = Marshal.AllocHGlobal(req);
      Marshal.WriteInt32(buf, IntPtr.Size == 8 ? 8 : 6);
      SetupDiGetDeviceInterfaceDetail(s, ref ifd, buf, req, out req, ref dd);
      string path = Marshal.PtrToStringUni(new IntPtr(buf.ToInt64() + 4));
      Marshal.FreeHGlobal(buf);
      var sb = new System.Text.StringBuilder(512);
      SetupDiGetDeviceInstanceId(s, ref dd, sb, 512, out req);
      if (string.Equals(sb.ToString(), instanceId, StringComparison.OrdinalIgnoreCase)) return path;
    }
  }
  public static string Cycle(string hubPath, int port) {
    var h = CreateFile(hubPath, 0xC0000000, 3, IntPtr.Zero, 3, 0, IntPtr.Zero);
    if (h.IsInvalid) return "open failed " + Marshal.GetLastWin32Error();
    byte[] p = new byte[8]; BitConverter.GetBytes(port).CopyTo(p, 0);
    int ret;
    bool ok = DeviceIoControl(h, 0x220444, p, 8, p, 8, out ret, IntPtr.Zero);
    h.Close();
    return ok ? "ok status=" + BitConverter.ToInt32(p, 4) : "ioctl failed " + Marshal.GetLastWin32Error();
  }
}
'@

function Prop($id, $key) { try { (Get-PnpDeviceProperty -InstanceId $id -KeyName $key -ErrorAction Stop).Data } catch { $null } }

try {
  $screen = Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -match '^USB\\VID_33C3&PID_77(88|91)\\' } | Select-Object -First 1
  $failed = @(Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -match '^USB\\VID_0000&PID_000[0-9A-F]\\' })
  if ($screen) { Log "screen already present: $($screen.InstanceId)" }
  if ($DryRun) {
    # Show that the port lookup works on the screen's own node (the same steps a failed node goes through).
    foreach ($d in @($screen) + $failed | Where-Object { $_ }) {
      $hub = Prop $d.InstanceId 'DEVPKEY_Device_Parent'
      $loc = Prop $d.InstanceId 'DEVPKEY_Device_LocationInfo'
      $m = [regex]::Match([string]$loc, 'Port_#(\d+)')
      $path = if ($hub) { [Hub]::PathFor($hub) } else { $null }
      Log "dry run: node $($d.InstanceId) location '$loc' port $(if ($m.Success) { [int]$m.Groups[1].Value } else { '?' }) hub $hub interface $(if ($path) { 'found' } else { 'NOT found' })"
    }
  }
  if (-not $failed) {
    if ($screen) { Log 'RESULT: OK'; return }
    if ($DryRun) { Log 'dry run: no failed USB device present, nothing would be cycled'; Log 'RESULT: OK (dry run)'; return }
    throw 'no failed USB device found and the screen is not present (check the cable / power)'
  }
  # Where was the screen before? Look at remembered (non-present) screen nodes.
  $hist = @(Get-PnpDevice -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -match '^USB\\VID_33C3&PID_77(88|91)\\' -and -not $_.Present })
  $histLoc = @($hist | ForEach-Object { Prop $_.InstanceId 'DEVPKEY_Device_LocationInfo' } | Where-Object { $_ })
  $histPar = @($hist | ForEach-Object { Prop $_.InstanceId 'DEVPKEY_Device_Parent' } | Where-Object { $_ })
  Log "failed nodes: $($failed.Count); remembered screen locations: $($histLoc -join ', ')"
  $targets = @($failed | Where-Object {
    $l = Prop $_.InstanceId 'DEVPKEY_Device_LocationInfo'
    $p = Prop $_.InstanceId 'DEVPKEY_Device_Parent'
    ($l -and $histLoc -contains $l -and (-not $histPar -or $histPar -contains $p))
  })
  if (-not $targets) { Log 'no history match, cycling all failed nodes'; $targets = $failed }
  foreach ($dev in $targets) {
    $hub = Prop $dev.InstanceId 'DEVPKEY_Device_Parent'
    $loc = Prop $dev.InstanceId 'DEVPKEY_Device_LocationInfo'
    $m = [regex]::Match([string]$loc, 'Port_#(\d+)')
    if (-not $m.Success) { Log "skip $($dev.InstanceId): no port in location '$loc'"; continue }
    $port = [int]$m.Groups[1].Value
    $path = [Hub]::PathFor($hub)
    if (-not $path) { Log "skip $($dev.InstanceId): hub interface for $hub not found"; continue }
    if ($DryRun) { Log "WOULD cycle $($dev.InstanceId) hub $hub port $port"; continue }
    Log "cycling $($dev.InstanceId) hub $hub port $port : $([Hub]::Cycle($path, $port))"
  }
  if ($DryRun) { Log 'RESULT: OK (dry run, nothing cycled)'; return }
  Start-Sleep 6
  $now = Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -match '^USB\\VID_33C3&PID_77(88|91)\\' } | Select-Object -First 1
  if (-not $now) { throw 'ports were cycled but the screen did not appear (try replugging the USB cable)' }
  Log "screen now: $($now.InstanceId) $($now.Status)"
  Log 'RESULT: OK'
} catch { Log "RESULT: ERROR $_" }
