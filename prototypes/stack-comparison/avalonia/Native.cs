using System.Runtime.InteropServices;
using System.Text.Json;
using Avalonia.Controls;

namespace Quote.Avalonia;

// Deliberately the same Win32 policy/region algorithm as slint/src/native.rs.
internal static class Native
{
    [StructLayout(LayoutKind.Sequential)] struct StylePair {public uint Old,New;}
    delegate nint Subclass(nint h,uint message,nint wp,nint lp,nuint id,nuint data);
    static readonly Subclass OverlayProcedure=OverlayMessage;
    [DllImport("comctl32.dll")] static extern bool SetWindowSubclass(nint h,Subclass proc,nuint id,nuint data);
    [DllImport("comctl32.dll")] static extern bool RemoveWindowSubclass(nint h,Subclass proc,nuint id);
    [DllImport("comctl32.dll")] static extern nint DefSubclassProc(nint h,uint message,nint wp,nint lp);
    static nint OverlayMessage(nint h,uint message,nint wp,nint lp,nuint id,nuint data)
    {
        if(message==0x21)return 3; // MA_NOACTIVATE, including intentional pointer interaction.
        if(message==0x82)RemoveWindowSubclass(h,OverlayProcedure,id);
        nint result=DefSubclassProc(h,message,wp,lp);
        if(message==0x7C&&wp.ToInt64()==-20){var style=Marshal.PtrToStructure<StylePair>(lp);style.New=(style.New|0x08000080u)&~0x00040000u;Marshal.StructureToPtr(style,lp,false);}
        return result;
    }
    [StructLayout(LayoutKind.Sequential)] internal struct Rect { public int Left, Top, Right, Bottom; }
    [DllImport("user32.dll")] internal static extern nint GetForegroundWindow();
    [DllImport("user32.dll", EntryPoint="GetWindowLongPtrW")] static extern nint GetWindowLong(nint h, int index);
    [DllImport("user32.dll", EntryPoint="SetWindowLongPtrW")] static extern nint SetWindowLong(nint h, int index, nint value);
    [DllImport("user32.dll")] static extern bool SetWindowPos(nint h, nint after, int x, int y, int cx, int cy, uint flags);
    [DllImport("user32.dll")] static extern bool GetClientRect(nint h, out Rect r);
    [DllImport("user32.dll")] static extern int SetWindowRgn(nint h, nint region, bool redraw);
    [DllImport("user32.dll")] static extern int GetWindowRgn(nint h, nint region);
    [DllImport("user32.dll")] internal static extern uint GetDpiForWindow(nint h);
    [DllImport("gdi32.dll")] static extern nint CreateRectRgn(int x, int y, int right, int bottom);
    [DllImport("gdi32.dll")] static extern int CombineRgn(nint output, nint first, nint second, int mode);
    [DllImport("gdi32.dll")] static extern bool DeleteObject(nint h);
    [DllImport("gdi32.dll")] static extern bool PtInRegion(nint h, int x, int y);

    internal static nint Handle(Window window) => window.TryGetPlatformHandle()?.Handle ?? 0;

    internal static void Overlay(Window window, string common, bool mascot)
    {
        nint h = Handle(window);
        if (h == 0) throw new InvalidOperationException("Window has no HWND");
        if(!SetWindowSubclass(h,OverlayProcedure,0x4b5651,0))throw new InvalidOperationException("SetWindowSubclass failed");
        long style = GetWindowLong(h, -20).ToInt64();
        SetWindowLong(h, -20, (nint)((style | 0x08000080L) & ~0x00040000L));
        SetWindowPos(h, (nint)(-1), 0, 0, 0, 0, 0x0010 | 0x0020 | 0x0001 | 0x0002);
        if (!mascot) return;
        using var mask = JsonDocument.Parse(File.ReadAllText(Path.Combine(common, "mask.json")));
        GetClientRect(h, out var size);
        double sx = size.Right / 150.0, sy = size.Bottom / 170.0;
        nint region = CreateRectRgn(0, 0, 0, 0);
        try
        {
            foreach (var rectangle in mask.RootElement.GetProperty("rects").EnumerateArray())
            {
                var v = rectangle.EnumerateArray().Select(n => n.GetInt32()).ToArray();
                nint part = CreateRectRgn((int)Math.Floor(v[0]*sx), (int)Math.Floor(v[1]*sy), (int)Math.Ceiling(v[2]*sx), (int)Math.Ceiling(v[3]*sy));
                CombineRgn(region, region, part, 2); DeleteObject(part);
            }
            if (SetWindowRgn(h, region, true) == 0) throw new InvalidOperationException("SetWindowRgn failed");
            region = 0; // Window now owns the native region.
        }
        finally { if (region != 0) DeleteObject(region); }
    }

    internal static object Inspect(Window window)
    {
        nint h = Handle(window); long s = GetWindowLong(h, -20).ToInt64();
        GetClientRect(h, out var size);
        nint r = CreateRectRgn(0,0,0,0);
        try
        {
            bool has = GetWindowRgn(h, r) > 0;
            return new { hwnd = h.ToInt64(), no_activate = (s & 0x08000000L) != 0, tool_window = (s & 0x80) != 0,
                topmost = (s & 8) != 0, dpi = GetDpiForWindow(h), width = size.Right, height = size.Bottom,
                region_present = has, transparent_corner_excluded = has && !PtInRegion(r, 1,1),
                solid_part_included = has && PtInRegion(r, size.Right*42/150, size.Bottom*60/170) };
        }
        finally { DeleteObject(r); }
    }
}
