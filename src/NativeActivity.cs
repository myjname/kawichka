using System;
using System.Diagnostics;
using System.Drawing;
using System.Runtime.InteropServices;

public sealed class NativeActivity:IDisposable {
 delegate IntPtr Hook(int code,IntPtr message,IntPtr data);
 readonly Hook mouseCallback,keyCallback;
 IntPtr mouseHook,keyHook;
 readonly Settings settings;
 readonly ActivityFilter filter;
 Point previous;
 bool hasPoint;
 public bool Available {get{return mouseHook!=IntPtr.Zero&&keyHook!=IntPtr.Zero;}}
 [StructLayout(LayoutKind.Sequential)]struct InputInfo{public uint Size,Tick;}
 [StructLayout(LayoutKind.Sequential)]struct MouseInfo{public int X,Y;public uint Data,Flags,Time;public UIntPtr Extra;}
 [DllImport("user32.dll")]static extern bool GetLastInputInfo(ref InputInfo info);
 [DllImport("user32.dll")]static extern IntPtr SetWindowsHookEx(int kind,Hook callback,IntPtr module,uint thread);
 [DllImport("user32.dll")]static extern bool UnhookWindowsHookEx(IntPtr hook);
 [DllImport("user32.dll")]static extern IntPtr CallNextHookEx(IntPtr hook,int code,IntPtr message,IntPtr data);
 [DllImport("kernel32.dll",CharSet=CharSet.Unicode)]static extern IntPtr GetModuleHandle(string name);
 public NativeActivity(Settings s){settings=s;var info=new InputInfo{Size=(uint)Marshal.SizeOf(typeof(InputInfo))};uint idle=GetLastInputInfo(ref info)?unchecked((uint)Environment.TickCount-info.Tick):0;filter=new ActivityFilter(DateTime.UtcNow.AddMilliseconds(-idle),settings);mouseCallback=Mouse;keyCallback=Key;
  IntPtr module=GetModuleHandle(null);mouseHook=SetWindowsHookEx(14,mouseCallback,module,0);keyHook=SetWindowsHookEx(13,keyCallback,module,0);
 }
 IntPtr Key(int code,IntPtr message,IntPtr data){if(code>=0&&(message.ToInt32()==0x100||message.ToInt32()==0x104))filter.Keyboard(DateTime.UtcNow);return CallNextHookEx(keyHook,code,message,data);}
 IntPtr Mouse(int code,IntPtr message,IntPtr data){if(code>=0){var now=DateTime.UtcNow;int kind=message.ToInt32();if(kind==0x200){var info=(MouseInfo)Marshal.PtrToStructure(data,typeof(MouseInfo));var point=new Point(info.X,info.Y);if(hasPoint){double dx=point.X-previous.X,dy=point.Y-previous.Y;filter.Move(now,Math.Sqrt(dx*dx+dy*dy));}previous=point;hasPoint=true;}else if(kind==0x201||kind==0x204||kind==0x207||kind==0x20B)filter.Click(now,settings);else if(kind==0x20A||kind==0x20E)filter.Scroll(now);}return CallNextHookEx(mouseHook,code,message,data);}
 public bool IsIdle(DateTime now){return Available&&filter.IsIdle(now,settings);}
 public bool HasRestedSince(DateTime utc){return Available&&filter.CompletedRestUtc>utc;}
 public void ObserveSuspension(DateTime startedUtc,DateTime resumedUtc){filter.ObserveSuspension(startedUtc,resumedUtc);}
 public void Dispose(){if(mouseHook!=IntPtr.Zero){UnhookWindowsHookEx(mouseHook);mouseHook=IntPtr.Zero;}if(keyHook!=IntPtr.Zero){UnhookWindowsHookEx(keyHook);keyHook=IntPtr.Zero;}}
}

public static class FullscreenDetector {
 [StructLayout(LayoutKind.Sequential)]struct Rect{public int Left,Top,Right,Bottom;}
 [DllImport("user32.dll")]static extern IntPtr GetForegroundWindow();
 [DllImport("user32.dll")]static extern bool GetWindowRect(IntPtr hwnd,out Rect rect);
 [DllImport("user32.dll")]static extern uint GetWindowThreadProcessId(IntPtr hwnd,out uint process);
 [DllImport("user32.dll",CharSet=CharSet.Unicode)]static extern int GetClassName(IntPtr hwnd,System.Text.StringBuilder name,int max);
 public static bool IsFullscreen(){IntPtr window=GetForegroundWindow();if(window==IntPtr.Zero)return false;uint pid;GetWindowThreadProcessId(window,out pid);if(pid==Process.GetCurrentProcess().Id)return false;var name=new System.Text.StringBuilder(128);GetClassName(window,name,128);if(name.ToString()=="Progman"||name.ToString()=="WorkerW"||name.ToString()=="Shell_TrayWnd")return false;Rect r;if(!GetWindowRect(window,out r))return false;var b=System.Windows.Forms.Screen.FromHandle(window).Bounds;return Math.Abs(r.Left-b.Left)<=2&&Math.Abs(r.Top-b.Top)<=2&&Math.Abs(r.Right-b.Right)<=2&&Math.Abs(r.Bottom-b.Bottom)<=2;}
}
