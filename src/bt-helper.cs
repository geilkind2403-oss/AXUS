// Compiled at runtime by main.js (ensureBtHelperCompiled) into bt-helper.exe,
// cached until this source file changes -- see the comment on
// getBluetoothStatus() in main.js for why this exists as a separate
// compiled helper instead of a PowerShell script: Windows PowerShell 5.1
// cannot subscribe to WinRT events (Register-ObjectEvent refuses them
// outright), and DeviceWatcher -- the only API for discovering nearby
// unpaired Bluetooth devices -- is entirely event-driven. Compiled C# has
// no such restriction, so discovery and pairing happen here instead.
//
// Usage:
//   bt-helper.exe scan <seconds>   -> JSON array of {"name","id"} found during the scan
//   bt-helper.exe pair <deviceId>  -> JSON {"ok":true} or {"ok":false,"error":"..."}
//
// Pairing auto-accepts "confirm only" / "just works" requests (how nearly
// all modern Bluetooth audio/input devices pair) with no PIN needed. A
// device that requires typing or confirming a PIN on this side is reported
// as a clear failure rather than silently hanging, since resuming that
// across a one-shot process invocation isn't practical here.
using System;
using System.Collections.Generic;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using Windows.Devices.Enumeration;
using Windows.Devices.Bluetooth;
using Windows.Devices.Bluetooth.Advertisement;
using Windows.Foundation;

public class BtHelper
{
    static string JsonEscape(string s)
    {
        if (s == null) return "";
        var sb = new StringBuilder();
        foreach (char c in s)
        {
            if (c == '"' || c == '\\') sb.Append('\\').Append(c);
            else if (c == '\n') sb.Append("\\n");
            else if (c < 0x20) sb.Append(' ');
            else sb.Append(c);
        }
        return sb.ToString();
    }

    static T Await<T>(IAsyncOperation<T> op, int timeoutMs)
    {
        var done = new ManualResetEventSlim(false);
        T result = default(T);
        Exception error = null;
        op.Completed = new AsyncOperationCompletedHandler<T>((o, status) => {
            try
            {
                if (status == AsyncStatus.Completed) result = o.GetResults();
                else error = new Exception("WinRT op status: " + status);
            }
            catch (Exception ex) { error = ex; }
            done.Set();
        });
        if (!done.Wait(timeoutMs)) throw new Exception("Timed out");
        if (error != null) throw error;
        return result;
    }

    static int Scan(int seconds)
    {
        var results = new List<string>();
        var seen = new HashSet<string>();

        // Classic Bluetooth devices, via the higher-level device-cache
        // watcher (also picks up BLE devices Windows has already cached
        // from earlier activity).
        string classicSelector = BluetoothDevice.GetDeviceSelectorFromPairingState(false);
        var classicWatcher = DeviceInformation.CreateWatcher(classicSelector);
        var classicStopped = new ManualResetEventSlim(false);
        classicWatcher.Added += new TypedEventHandler<DeviceWatcher, DeviceInformation>((w, info) => {
            if (string.IsNullOrEmpty(info.Name)) return; // unnamed devices aren't useful to show/pair
            lock (results)
            {
                if (seen.Add(info.Id)) results.Add("{\"name\":\"" + JsonEscape(info.Name) + "\",\"id\":\"" + JsonEscape(info.Id) + "\"}");
            }
        });
        classicWatcher.Stopped += new TypedEventHandler<DeviceWatcher, object>((w, o) => { classicStopped.Set(); });

        // Raw BLE advertisement packets -- most phones and modern
        // accessories are BLE-first, and listening to broadcasts directly
        // finds them faster/more reliably than the device-cache watcher
        // above, which can lag behind or miss devices it hasn't seen yet.
        var seenAddresses = new HashSet<ulong>();
        var bleWatcher = new BluetoothLEAdvertisementWatcher();
        bleWatcher.ScanningMode = BluetoothLEScanningMode.Active;
        bleWatcher.Received += new TypedEventHandler<BluetoothLEAdvertisementWatcher, BluetoothLEAdvertisementReceivedEventArgs>((w, args) => {
            string name = args.Advertisement.LocalName;
            if (string.IsNullOrEmpty(name)) return; // anonymous/unnamed BLE beacons aren't useful to show or pair
            lock (seenAddresses)
            {
                if (!seenAddresses.Add(args.BluetoothAddress)) return;
            }
            try
            {
                // Resolves the raw advertisement to a proper pairable
                // DeviceInformation id (a BluetoothAddress alone isn't one).
                var bleDevice = Await(BluetoothLEDevice.FromBluetoothAddressAsync(args.BluetoothAddress), 5000);
                if (bleDevice == null) return;
                lock (results)
                {
                    if (seen.Add(bleDevice.DeviceId)) results.Add("{\"name\":\"" + JsonEscape(name) + "\",\"id\":\"" + JsonEscape(bleDevice.DeviceId) + "\"}");
                }
            }
            catch { /* couldn't resolve this one, skip it */ }
        });

        classicWatcher.Start();

        // Some Bluetooth adapters/drivers (seen with certain Realtek USB
        // dongles) refuse BluetoothLEAdvertisementWatcher outright, in
        // both Active and Passive mode, throwing here rather than on a
        // later event. That must not take the classic watcher down with
        // it -- classic discovery (which also picks up already-cached BLE
        // devices) can still work fine on such hardware.
        bool bleStarted = false;
        try
        {
            bleWatcher.Start();
            bleStarted = true;
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine("BLE advertisement watcher unavailable, continuing with classic scan only: " + ex.Message);
        }

        Thread.Sleep(seconds * 1000);
        classicWatcher.Stop();
        if (bleStarted)
        {
            try { bleWatcher.Stop(); } catch { /* already stopped/faulted, nothing to do */ }
        }
        classicStopped.Wait(5000);

        Console.WriteLine("[" + string.Join(",", results) + "]");
        return 0;
    }

    static int Pair(string deviceId)
    {
        try
        {
            var info = Await(DeviceInformation.CreateFromIdAsync(deviceId), 10000);
            var custom = info.Pairing.Custom;

            custom.PairingRequested += new TypedEventHandler<DeviceInformationCustomPairing, DevicePairingRequestedEventArgs>((sender, args) => {
                // ConfirmOnly / just-works: accept immediately, no PIN needed.
                // Anything that needs a PIN typed or confirmed here is left
                // unhandled -- Deferral is never taken, so it will fail
                // cleanly rather than hang.
                if (args.PairingKind == DevicePairingKinds.ConfirmOnly)
                {
                    args.Accept();
                }
            });

            var result = Await(
                custom.PairAsync(DevicePairingKinds.ConfirmOnly, DevicePairingProtectionLevel.Default),
                20000
            );

            if (result.Status == DevicePairingResultStatus.Paired || result.Status == DevicePairingResultStatus.AlreadyPaired)
            {
                Console.WriteLine("{\"ok\":true}");
                return 0;
            }
            Console.WriteLine("{\"ok\":false,\"error\":\"" + JsonEscape(result.Status.ToString()) + "\"}");
            return 0;
        }
        catch (Exception ex)
        {
            Console.WriteLine("{\"ok\":false,\"error\":\"" + JsonEscape(ex.Message) + "\"}");
            return 0;
        }
    }

    public static int Main(string[] args)
    {
        try
        {
            if (args.Length >= 2 && args[0] == "scan") return Scan(int.Parse(args[1]));
            if (args.Length >= 2 && args[0] == "pair") return Pair(args[1]);
            Console.Error.WriteLine("Usage: bt-helper.exe scan <seconds> | pair <deviceId>");
            return 1;
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine(ex.ToString());
            return 1;
        }
    }
}
