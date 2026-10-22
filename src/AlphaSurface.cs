using System;
using System.ComponentModel;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;

// A reusable premultiplied-alpha DIB. GDI+ renders directly into the selected
// bitmap; no chroma key, screen capture or per-frame bitmap allocation.
public sealed class AlphaSurface:IDisposable {
 [StructLayout(LayoutKind.Sequential)]struct Header{public uint Size;public int Width,Height;public ushort Planes,Bits;public uint Compression,Bytes;public int X,Y;public uint Used,Important;}
 [StructLayout(LayoutKind.Sequential)]struct Info{public Header Header;public uint Color;}
 [StructLayout(LayoutKind.Sequential)]struct Pair{public int X,Y;public Pair(int x,int y){X=x;Y=y;}}
 [StructLayout(LayoutKind.Sequential,Pack=1)]struct Blend{public byte Operation,Flags,Alpha,Format;}
 [DllImport("gdi32.dll",SetLastError=true)]static extern IntPtr CreateCompatibleDC(IntPtr dc);
 [DllImport("gdi32.dll",SetLastError=true)]static extern IntPtr CreateDIBSection(IntPtr dc,ref Info info,uint usage,out IntPtr bits,IntPtr section,uint offset);
 [DllImport("gdi32.dll",SetLastError=true)]static extern IntPtr SelectObject(IntPtr dc,IntPtr value);
 [DllImport("gdi32.dll")]static extern bool DeleteObject(IntPtr value);
 [DllImport("gdi32.dll")]static extern bool DeleteDC(IntPtr dc);
 [DllImport("user32.dll",SetLastError=true)]static extern bool UpdateLayeredWindow(IntPtr window,IntPtr screen,ref Pair position,ref Pair size,IntPtr source,ref Pair origin,uint key,ref Blend blend,uint flags);
 IntPtr dc,bitmap,previous,bits;
 public Bitmap Frame {get;private set;}
 public AlphaSurface(int width,int height){
  if(width<1||height<1||width>2048||height>2048)throw new ArgumentOutOfRangeException("width");
  try{
   dc=CreateCompatibleDC(IntPtr.Zero);if(dc==IntPtr.Zero)throw new Win32Exception();
   var info=new Info{Header=new Header{Size=(uint)Marshal.SizeOf(typeof(Header)),Width=width,Height=-height,Planes=1,Bits=32,Bytes=(uint)(width*height*4)}};
   bitmap=CreateDIBSection(dc,ref info,0,out bits,IntPtr.Zero,0);if(bitmap==IntPtr.Zero||bits==IntPtr.Zero)throw new Win32Exception();
   previous=SelectObject(dc,bitmap);if(previous==IntPtr.Zero||previous==new IntPtr(-1))throw new Win32Exception();
   Frame=new Bitmap(width,height,width*4,PixelFormat.Format32bppPArgb,bits);
  }catch{Dispose();throw;}
 }
 public void Present(IntPtr window,Point location){
  if(Frame==null)throw new ObjectDisposedException("AlphaSurface");
  var point=new Pair(location.X,location.Y);var size=new Pair(Frame.Width,Frame.Height);var origin=new Pair(0,0);var blend=new Blend{Alpha=255,Format=1};
  if(!UpdateLayeredWindow(window,IntPtr.Zero,ref point,ref size,dc,ref origin,0,ref blend,2))throw new Win32Exception();
 }
 public void Dispose(){
  if(Frame!=null){Frame.Dispose();Frame=null;}
  if(dc!=IntPtr.Zero&&previous!=IntPtr.Zero&&previous!=new IntPtr(-1))SelectObject(dc,previous);
  previous=IntPtr.Zero;if(bitmap!=IntPtr.Zero){DeleteObject(bitmap);bitmap=IntPtr.Zero;}bits=IntPtr.Zero;
  if(dc!=IntPtr.Zero){DeleteDC(dc);dc=IntPtr.Zero;}
 }
}
