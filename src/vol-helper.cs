// Tiny command-line helper for the Windows master volume (Core Audio,
// IAudioEndpointVolume). Compiled on first use by main.js with the csc.exe
// that ships with Windows, exactly like bt-helper.cs -- no SDK needed.
//
//   vol-helper get          -> {"level":42,"muted":false}
//   vol-helper set 55       -> sets 0..100, prints the new state
//   vol-helper mute 1|0     -> mutes / unmutes, prints the new state
using System;
using System.Runtime.InteropServices;

[ComImport, Guid("BCDE0395-E52F-467C-8E3D-C4579291692E")]
class MMDeviceEnumeratorCom { }

[Guid("A95664D2-9614-4F35-A746-DE8DB63617E6"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IMMDeviceEnumerator {
  int NotImpl1();
  [PreserveSig] int GetDefaultAudioEndpoint(int dataFlow, int role, out IMMDevice ppDevice);
}

[Guid("D666063F-1587-4E43-81F1-B948E807363F"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IMMDevice {
  [PreserveSig] int Activate(ref Guid iid, int dwClsCtx, IntPtr pActivationParams, [MarshalAs(UnmanagedType.IUnknown)] out object ppInterface);
}

[Guid("5CDF2C82-841E-4546-9722-0CF74078229A"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IAudioEndpointVolume {
  int NotImpl1(); // RegisterControlChangeNotify
  int NotImpl2(); // UnregisterControlChangeNotify
  int NotImpl3(); // GetChannelCount
  int NotImpl4(); // SetMasterVolumeLevel
  [PreserveSig] int SetMasterVolumeLevelScalar(float fLevel, ref Guid pguidEventContext);
  int NotImpl5(); // GetMasterVolumeLevel
  [PreserveSig] int GetMasterVolumeLevelScalar(out float pfLevel);
  int NotImpl6(); // SetChannelVolumeLevel
  int NotImpl7(); // SetChannelVolumeLevelScalar
  int NotImpl8(); // GetChannelVolumeLevel
  int NotImpl9(); // GetChannelVolumeLevelScalar
  [PreserveSig] int SetMute([MarshalAs(UnmanagedType.Bool)] bool bMute, ref Guid pguidEventContext);
  [PreserveSig] int GetMute(out bool pbMute);
}

static class VolHelper {
  static IAudioEndpointVolume Endpoint() {
    var enumerator = (IMMDeviceEnumerator)(new MMDeviceEnumeratorCom());
    IMMDevice device;
    Marshal.ThrowExceptionForHR(enumerator.GetDefaultAudioEndpoint(0 /* eRender */, 1 /* eMultimedia */, out device));
    Guid iid = typeof(IAudioEndpointVolume).GUID;
    object o;
    Marshal.ThrowExceptionForHR(device.Activate(ref iid, 23 /* CLSCTX_ALL */, IntPtr.Zero, out o));
    return (IAudioEndpointVolume)o;
  }

  static int Main(string[] args) {
    try {
      var ep = Endpoint();
      Guid ctx = Guid.Empty;
      string cmd = args.Length > 0 ? args[0] : "get";
      if (cmd == "set" && args.Length > 1) {
        int v = Math.Max(0, Math.Min(100, int.Parse(args[1])));
        ep.SetMasterVolumeLevelScalar(v / 100f, ref ctx);
        if (v > 0) ep.SetMute(false, ref ctx);
      } else if (cmd == "mute" && args.Length > 1) {
        ep.SetMute(args[1] == "1", ref ctx);
      }
      float level; bool muted;
      ep.GetMasterVolumeLevelScalar(out level);
      ep.GetMute(out muted);
      Console.WriteLine("{\"level\":" + (int)Math.Round(level * 100) + ",\"muted\":" + (muted ? "true" : "false") + "}");
      return 0;
    } catch (Exception e) {
      Console.WriteLine("{\"error\":\"" + e.Message.Replace("\\", "\\\\").Replace("\"", "'") + "\"}");
      return 1;
    }
  }
}
