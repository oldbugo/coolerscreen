# Elevated (CoolerScreen launches it via a UAC prompt): reconnect a screen that Windows failed to enumerate
# ("Unknown USB Device (Device Descriptor Request Failed)", instance USB\VID_0000&PID_0002\...).
# It power-cycles the single USB hub port the broken device sits on (IOCTL_USB_HUB_CYCLE_PORT), which makes the
# screen enumerate again. -DryRun only lists what it would cycle (no admin rights needed).
# Only one failed port with an exact remembered screen parent/location pair is eligible. Never reset unrelated USB ports.
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

function Select-CoolerReconnectTargets($FailedNodes, $ScreenHistory) {
  # Keep the parent and location together. Separate lists could match the parent of
  # one remembered screen with the location of a different screen.
  @($FailedNodes | Where-Object {
    $candidate = $_
    $candidate.Parent -and $candidate.Location -and @($ScreenHistory | Where-Object {
      $_.Parent -eq $candidate.Parent -and $_.Location -eq $candidate.Location
    }).Count -gt 0
  })
}

try {
  $screen = Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -match '^USB\\VID_33C3&PID_77(88|91)\\' } | Select-Object -First 1
  $failed = @(Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -match '^USB\\VID_0000&PID_000[0-9A-F]\\' })
  if ($screen) {
    Log "screen already present: $($screen.InstanceId); no USB port reset needed"
    Log 'RESULT: OK'
    return
  }
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
  $history = @($hist | ForEach-Object {
    [pscustomobject]@{ Parent = Prop $_.InstanceId 'DEVPKEY_Device_Parent'; Location = Prop $_.InstanceId 'DEVPKEY_Device_LocationInfo' }
  })
  $candidates = @($failed | ForEach-Object {
    [pscustomobject]@{ Node = $_; Parent = Prop $_.InstanceId 'DEVPKEY_Device_Parent'; Location = Prop $_.InstanceId 'DEVPKEY_Device_LocationInfo' }
  })
  Log "failed nodes: $($failed.Count); remembered screen locations: $(($history | ForEach-Object { $_.Location }) -join ', ')"
  $targets = @(Select-CoolerReconnectTargets $candidates $history)
  if ($targets.Count -ne 1) {
    throw "cannot identify one failed cooler-screen USB port safely (matched $($targets.Count)). No USB ports were reset. Unplug and reconnect the screen's USB cable."
  }
  foreach ($dev in $targets) {
    $hub = $dev.Parent
    $loc = $dev.Location
    $nodeId = $dev.Node.InstanceId
    $m = [regex]::Match([string]$loc, 'Port_#(\d+)')
    if (-not $m.Success) { throw "no port in screen location '$loc'; no USB port reset" }
    $port = [int]$m.Groups[1].Value
    $path = [Hub]::PathFor($hub)
    if (-not $path) { throw 'screen USB hub interface not found; no USB port reset' }
    if ($DryRun) { Log "WOULD cycle $nodeId hub $hub port $port"; continue }
    $cycleResult = [Hub]::Cycle($path, $port)
    Log "cycling $nodeId hub $hub port $port : $cycleResult"
    if ($cycleResult -ne 'ok status=0') { throw "Windows could not reset the screen's USB port ($cycleResult)" }
  }
  if ($DryRun) { Log 'RESULT: OK (dry run, nothing cycled)'; return }
  $deadline = [DateTime]::UtcNow.AddSeconds(20)
  do {
    Start-Sleep 2
    $now = Get-PnpDevice -PresentOnly -ErrorAction SilentlyContinue | Where-Object { $_.InstanceId -match '^USB\\VID_33C3&PID_77(88|91)\\' -and $_.Status -eq 'OK' } | Select-Object -First 1
  } while (-not $now -and [DateTime]::UtcNow -lt $deadline)
  if (-not $now) { throw "the screen's USB port was reset once, but Windows still cannot recognise the screen. Unplug and reconnect only the screen's USB cable; a port reset may not remove its power." }
  Log "screen now: $($now.InstanceId) $($now.Status)"
  Log 'RESULT: OK'
} catch { Log "RESULT: ERROR $_" }
