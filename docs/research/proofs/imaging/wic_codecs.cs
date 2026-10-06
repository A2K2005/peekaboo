// Proves which WIC decoders and encoders exist on this PC, which are built in vs added by Store extensions, and whether HEVC/AV1 decoder MFTs exist.
// Build: csc /out:wic_codecs.exe wic_codecs.cs WicInterop.cs   Run: wic_codecs.exe
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;

static class WicCodecs
{
    const uint WICDecoder = 1, WICEncoder = 2;
    const uint EnumDefault = 0, EnumBuiltInOnly = 0x20000000;

    static void Main()
    {
        var f = Wic.CreateFactory();
        foreach (var type in new[] { WICDecoder, WICEncoder })
        {
            var builtIn = new HashSet<Guid>();
            foreach (var c in List(f, type, EnumBuiltInOnly)) { Guid id; c.GetCLSID(out id); builtIn.Add(id); }
            Console.WriteLine(type == WICDecoder ? "== WIC decoders" : "== WIC encoders");
            foreach (var c in List(f, type, EnumDefault))
            {
                Guid id; c.GetCLSID(out id);
                Console.WriteLine("{0,-9} {1,-40} ver {2,-12} {3}",
                    builtIn.Contains(id) ? "built-in" : "EXTENSION",
                    Wic.Str(c.GetFriendlyName), Wic.Str(c.GetVersion), Wic.Str(c.GetFileExtensions));
            }
        }

        Console.WriteLine("== CreateEncoder by container format");
        TryEncoder(f, "WebP", "e094b0e2-67f2-45b3-b0ea-115337ca7cf3");
        TryEncoder(f, "HEIF", "e1e62521-6787-405b-a339-500715b5763f");

        Console.WriteLine("== Media Foundation video decoders (MFTEnumEx, MFT_ENUM_FLAG_ALL)");
        Mf.Startup(0x20070, 0);
        Mf.ListDecoders("HEVC", "43564548-0000-0010-8000-00AA00389B71");
        Mf.ListDecoders("AV1 ", "31305641-0000-0010-8000-00AA00389B71");
    }

    static IEnumerable<IWICBitmapCodecInfo> List(IWICImagingFactory f, uint type, uint options)
    {
        IEnumUnknown e;
        f.CreateComponentEnumerator(type, options, out e);
        object o; uint got;
        while (e.Next(1, out o, out got) == 0 && got == 1) yield return (IWICBitmapCodecInfo)o;
    }

    static void TryEncoder(IWICImagingFactory f, string name, string container)
    {
        var g = new Guid(container);
        IntPtr enc;
        int hr = f.CreateEncoder(ref g, IntPtr.Zero, out enc);
        Console.WriteLine("{0}: hr=0x{1:X8} {2}", name, hr, hr == 0 ? "encoder available" : "no encoder");
        if (enc != IntPtr.Zero) Marshal.Release(enc);
    }
}

static class Mf
{
    [DllImport("mfplat.dll")] public static extern int MFStartup(uint version, uint flags);
    [DllImport("mfplat.dll")] static extern int MFTEnumEx(Guid category, uint flags, ref TypeInfo input, IntPtr output, out IntPtr activates, out uint count);
    [StructLayout(LayoutKind.Sequential)] struct TypeInfo { public Guid Major, Sub; }

    [ComImport, Guid("7FEE9E9A-4A89-47a6-899C-B6A53A70FB67"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IMFActivate
    {
        void _GetItem(); void _GetItemType(); void _CompareItem(); void _Compare(); void _GetUINT32(); void _GetUINT64();
        void _GetDouble(); void _GetGUID(); void _GetStringLength(); void _GetString();
        [PreserveSig] int GetAllocatedString(ref Guid key, [MarshalAs(UnmanagedType.LPWStr)] out string value, out uint length);
    }

    public static void Startup(uint v, uint f) { MFStartup(v, f); }

    public static void ListDecoders(string label, string subtype)
    {
        var ti = new TypeInfo { Major = new Guid("73646976-0000-0010-8000-00AA00389B71"), Sub = new Guid(subtype) };
        IntPtr arr; uint n;
        int hr = MFTEnumEx(new Guid("d6c02d4b-6833-45b4-971a-05a4b04bab91"), 0x3F, ref ti, IntPtr.Zero, out arr, out n);
        Console.WriteLine("{0}: hr=0x{1:X8} count={2}", label, hr, n);
        var nameKey = new Guid("314ffbae-5b41-4c95-9c19-4e7d586face3"); // MFT_FRIENDLY_NAME_Attribute
        for (int i = 0; i < n; i++)
        {
            var a = (IMFActivate)Marshal.GetObjectForIUnknown(Marshal.ReadIntPtr(arr, i * IntPtr.Size));
            string name; uint len;
            Console.WriteLine("   {0}", a.GetAllocatedString(ref nameKey, out name, out len) == 0 ? name : "(no name)");
        }
    }
}
