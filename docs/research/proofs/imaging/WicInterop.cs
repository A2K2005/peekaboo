// Minimal WIC COM interop (C# 5) shared by the imaging proofs. Vtable order copied from wincodec.h, SDK 10.0.26100.
// Placeholder methods (no parameters) only keep vtable slots in order. Never call them.
using System;
using System.Runtime.InteropServices;
using System.Text;

static class Wic
{
    public static readonly Guid CLSID_Factory = new Guid("317d06e8-5f24-433d-bdf7-79ce68d8abc2"); // WICImagingFactory2
    public static readonly Guid PBGRA32 = new Guid("6fddc324-4e03-4bfe-b185-3d77768dc910");
    public static readonly Guid BGR24 = new Guid("6fddc324-4e03-4bfe-b185-3d77768dc90c");
    public const uint GENERIC_READ = 0x80000000;

    public static IWICImagingFactory CreateFactory()
    {
        return (IWICImagingFactory)Activator.CreateInstance(Type.GetTypeFromCLSID(CLSID_Factory));
    }

    public delegate void StrGetter(uint cch, StringBuilder buf, out uint actual);

    // Two-call WIC string pattern: ask for the length, then fill the buffer.
    public static string Str(StrGetter get)
    {
        uint n;
        get(0, null, out n);
        if (n == 0) return "";
        var buf = new StringBuilder((int)n);
        get(n, buf, out n);
        return buf.ToString();
    }
}

[ComImport, Guid("ec5ec8a9-c395-4314-9c77-54d7a935ff70"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IWICImagingFactory
{
    [PreserveSig] int CreateDecoderFromFilename([MarshalAs(UnmanagedType.LPWStr)] string name, IntPtr vendor, uint access, int options, out IWICBitmapDecoder decoder);
    void _CreateDecoderFromStream();
    void _CreateDecoderFromFileHandle();
    void _CreateComponentInfo();
    [PreserveSig] int CreateDecoder(ref Guid containerFormat, IntPtr vendor, out IntPtr decoder);
    [PreserveSig] int CreateEncoder(ref Guid containerFormat, IntPtr vendor, out IntPtr encoder);
    void _CreatePalette();
    void CreateFormatConverter(out IWICFormatConverter converter);
    void CreateBitmapScaler(out IWICBitmapScaler scaler);
    void _CreateBitmapClipper();
    void _CreateBitmapFlipRotator();
    void _CreateStream();
    void _CreateColorContext();
    void _CreateColorTransformer();
    void _CreateBitmap();
    void _CreateBitmapFromSource();
    void _CreateBitmapFromSourceRect();
    void _CreateBitmapFromMemory();
    void _CreateBitmapFromHBITMAP();
    void _CreateBitmapFromHICON();
    void CreateComponentEnumerator(uint componentTypes, uint options, out IEnumUnknown enumerator);
}

[ComImport, Guid("00000100-0000-0000-C000-000000000046"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IEnumUnknown
{
    [PreserveSig] int Next(uint celt, [MarshalAs(UnmanagedType.IUnknown)] out object item, out uint fetched);
}

[ComImport, Guid("E87A44C4-B76E-4c47-8B09-298EB12A2714"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IWICBitmapCodecInfo
{
    void GetComponentType(out int type);
    void GetCLSID(out Guid clsid);
    void GetSigningStatus(out uint status);
    void GetAuthor(uint cch, [MarshalAs(UnmanagedType.LPWStr)] StringBuilder buf, out uint actual);
    void GetVendorGUID(out Guid vendor);
    void GetVersion(uint cch, [MarshalAs(UnmanagedType.LPWStr)] StringBuilder buf, out uint actual);
    void GetSpecVersion(uint cch, [MarshalAs(UnmanagedType.LPWStr)] StringBuilder buf, out uint actual);
    void GetFriendlyName(uint cch, [MarshalAs(UnmanagedType.LPWStr)] StringBuilder buf, out uint actual);
    void GetContainerFormat(out Guid format);
    void _GetPixelFormats();
    void _GetColorManagementVersion();
    void _GetDeviceManufacturer();
    void _GetDeviceModels();
    void GetMimeTypes(uint cch, [MarshalAs(UnmanagedType.LPWStr)] StringBuilder buf, out uint actual);
    void GetFileExtensions(uint cch, [MarshalAs(UnmanagedType.LPWStr)] StringBuilder buf, out uint actual);
}

[ComImport, Guid("9EDDE9E7-8DEE-47ea-99DF-E6FAF2ED44BF"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IWICBitmapDecoder
{
    void _QueryCapability();
    void _Initialize();
    void GetContainerFormat(out Guid format);
    void _GetDecoderInfo();
    void _CopyPalette();
    void _GetMetadataQueryReader();
    void _GetPreview();
    void _GetColorContexts();
    void _GetThumbnail();
    void GetFrameCount(out uint count);
    void GetFrame(uint index, out IWICBitmapFrameDecode frame);
}

[ComImport, Guid("00000120-a8f2-4877-ba0a-fd2b6645fb94"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IWICBitmapSource
{
    void GetSize(out uint width, out uint height);
    void GetPixelFormat(out Guid format);
    void GetResolution(out double dpiX, out double dpiY);
    void _CopyPalette();
    void CopyPixels(IntPtr rect, uint stride, uint bufferSize, IntPtr buffer);
}

[ComImport, Guid("3B16811B-6A43-4ec9-A813-3D930C13B940"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IWICBitmapFrameDecode
{
    void GetSize(out uint width, out uint height);
    void GetPixelFormat(out Guid format);
    void GetResolution(out double dpiX, out double dpiY);
    void _CopyPalette();
    void CopyPixels(IntPtr rect, uint stride, uint bufferSize, IntPtr buffer);
    [PreserveSig] int GetMetadataQueryReader(out IWICMetadataQueryReader reader);
    [PreserveSig] int GetColorContexts(uint count, IntPtr contexts, out uint actual);
    [PreserveSig] int GetThumbnail(out IWICBitmapSource thumbnail);
}

[ComImport, Guid("3B16811B-6A43-4ec9-B713-3D5A0C13B940"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IWICBitmapSourceTransform
{
    void CopyPixels(IntPtr rect, uint width, uint height, ref Guid dstFormat, int transform, uint stride, uint bufferSize, IntPtr buffer);
    void GetClosestSize(ref uint width, ref uint height);
    void GetClosestPixelFormat(ref Guid format);
    void DoesSupportTransform(int transform, [MarshalAs(UnmanagedType.Bool)] out bool supported);
}

[ComImport, Guid("00000302-a8f2-4877-ba0a-fd2b6645fb94"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IWICBitmapScaler
{
    void GetSize(out uint width, out uint height);
    void GetPixelFormat(out Guid format);
    void GetResolution(out double dpiX, out double dpiY);
    void _CopyPalette();
    void CopyPixels(IntPtr rect, uint stride, uint bufferSize, IntPtr buffer);
    void Initialize([MarshalAs(UnmanagedType.IUnknown)] object source, uint width, uint height, int mode);
}

[ComImport, Guid("00000301-a8f2-4877-ba0a-fd2b6645fb94"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IWICFormatConverter
{
    void GetSize(out uint width, out uint height);
    void GetPixelFormat(out Guid format);
    void GetResolution(out double dpiX, out double dpiY);
    void _CopyPalette();
    void CopyPixels(IntPtr rect, uint stride, uint bufferSize, IntPtr buffer);
    void Initialize([MarshalAs(UnmanagedType.IUnknown)] object source, ref Guid dstFormat, int dither, IntPtr palette, double alphaThreshold, int paletteType);
}

[ComImport, Guid("30989668-E1C9-4597-B395-458EEDB808DF"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface IWICMetadataQueryReader
{
    void _GetContainerFormat();
    void _GetLocation();
    [PreserveSig] int GetMetadataByName([MarshalAs(UnmanagedType.LPWStr)] string name, ref PropVariant value);
}

[StructLayout(LayoutKind.Explicit, Size = 24)]
struct PropVariant
{
    [FieldOffset(0)] public ushort vt;
    [FieldOffset(8)] public ushort uiVal;
}
