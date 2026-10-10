# Elevated: power-cycle the single USB hub port that a failed device sits on (IOCTL_USB_HUB_CYCLE_PORT).
# Default target: the screen's port, found from the "Unknown USB Device" node with VID_0000 or VID_33C3.
param([string]$Root = (Split-Path -Parent $PSScriptRoot))
$log = Join-Path $Root 'work\cycle-port.log'
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
try {
  $dev = Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -match '^USB\\VID_(0000|33C3)&PID_[0-9A-F]{4}\\' } | Select-Object -First 1
  if (-not $dev) { throw 'no screen / failed device found' }
  $hub = (Get-PnpDeviceProperty -InstanceId $dev.InstanceId -KeyName DEVPKEY_Device_Parent).Data
  $loc = (Get-PnpDeviceProperty -InstanceId $dev.InstanceId -KeyName DEVPKEY_Device_LocationInfo).Data
  $port = [int]([regex]::Match($loc, 'Port_#(\d+)').Groups[1].Value)
  Log "device $($dev.InstanceId) hub $hub location '$loc' port $port"
  $path = [Hub]::PathFor($hub)
  if (-not $path) { throw "hub interface not found" }
  Log "hub path $path"
  Log ([Hub]::Cycle($path, $port))
  Start-Sleep 6
  Get-PnpDevice -PresentOnly | Where-Object { $_.InstanceId -match 'VID_33C3|VID_0000' } | ForEach-Object { Log "$($_.Status) $($_.Class) $($_.FriendlyName) $($_.InstanceId)" }
} catch { Log "ERROR $_" }
