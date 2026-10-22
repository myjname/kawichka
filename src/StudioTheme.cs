using System;
using System.Drawing;
using System.IO;
using System.Security;
using System.Runtime.InteropServices;
using System.Windows.Forms;
using Microsoft.Win32;

public sealed class StudioPalette {
 public Color Background,Sidebar,Surface,Ink,Muted,Accent,Soft,Border,Hover,AccentHover,OnAccent,Track;
 public static StudioPalette Create(bool dark){return dark?new StudioPalette{
  Background=Color.FromArgb(25,27,36),Sidebar=Color.FromArgb(31,33,45),Surface=Color.FromArgb(35,38,51),
  Ink=Color.FromArgb(239,240,249),Muted=Color.FromArgb(169,175,197),Accent=Color.FromArgb(183,165,255),
  Soft=Color.FromArgb(57,48,82),Border=Color.FromArgb(68,73,93),Hover=Color.FromArgb(48,51,69),
  AccentHover=Color.FromArgb(203,190,255),OnAccent=Color.FromArgb(31,24,49),Track=Color.FromArgb(86,92,113)
 }:new StudioPalette{
  Background=Color.FromArgb(248,249,252),Sidebar=Color.FromArgb(242,242,249),Surface=Color.White,
  Ink=Color.FromArgb(35,40,61),Muted=Color.FromArgb(112,118,139),Accent=Color.FromArgb(112,91,225),
  Soft=Color.FromArgb(234,230,251),Border=Color.FromArgb(222,225,235),Hover=Color.FromArgb(231,233,242),
  AccentHover=Color.FromArgb(95,75,203),OnAccent=Color.White,Track=Color.FromArgb(204,209,223)
 };}
}

public static partial class StudioColors {
 static StudioPalette palette;
 public static string Mode {get;private set;}
 public static bool IsDark {get;private set;}
 static StudioColors(){IsDark=ReadSystemDark();palette=StudioPalette.Create(IsDark);Mode="system";}
 // A replaceable reader keeps Windows preference handling testable without
 // changing the user's registry or desktop theme.
 public static Func<bool> SystemDarkReader=ReadSystemDark;
 public static Color Background{get{return palette.Background;}} public static Color Sidebar{get{return palette.Sidebar;}}
 public static Color Surface{get{return palette.Surface;}} public static Color Ink{get{return palette.Ink;}}
 public static Color Muted{get{return palette.Muted;}} public static Color Accent{get{return palette.Accent;}}
 public static Color Soft{get{return palette.Soft;}} public static Color Border{get{return palette.Border;}}
 public static Color Hover{get{return palette.Hover;}} public static Color AccentHover{get{return palette.AccentHover;}}
 public static Color OnAccent{get{return palette.OnAccent;}} public static Color Track{get{return palette.Track;}}
 public static bool ReadSystemDark(){try{using(var key=Registry.CurrentUser.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")){return key!=null&&Convert.ToInt32(key.GetValue("AppsUseLightTheme",1))==0;}}catch(SecurityException){return false;}catch(UnauthorizedAccessException){return false;}catch(IOException){return false;}}
 public static void SetMode(string mode){
  Mode=mode=="light"||mode=="dark"?mode:"system";bool dark=Mode=="dark"||(Mode=="system"&&SystemDarkReader());
  var old=palette;palette=StudioPalette.Create(dark);IsDark=dark;
  foreach(Form form in Application.OpenForms)if(!(form is PetForm))Apply(form,old);
 }
 public static void RefreshSystem(){if(Mode=="system")SetMode("system");}
 public static void PrepareMenu(ContextMenuStrip menu){menu.Renderer=new ToolStripProfessionalRenderer(new NightMenuColors());menu.Opening+=delegate{ThemeMenu(menu);};ThemeMenu(menu);}
 static void ThemeMenu(ToolStrip menu){menu.BackColor=Surface;menu.ForeColor=Ink;foreach(ToolStripItem item in menu.Items){item.BackColor=Surface;item.ForeColor=Ink;var branch=item as ToolStripDropDownItem;if(branch!=null&&branch.HasDropDownItems){branch.DropDown.Renderer=menu.Renderer;ThemeMenu(branch.DropDown);}}}
 [DllImport("uxtheme.dll",CharSet=CharSet.Unicode)]static extern int SetWindowTheme(IntPtr handle,string app,string classes);
 public static void NativeTheme(Control root){
  if(root.IsHandleCreated&&(root is ComboBox||root is ScrollableControl||root is NumericUpDown||root is ListView||root is ProgressBar))try{SetWindowTheme(root.Handle,IsDark?"DarkMode_Explorer":"Explorer",null);}catch(DllNotFoundException){}catch(EntryPointNotFoundException){}
  foreach(Control child in root.Controls)NativeTheme(child);
 }
 static Color BackgroundColor(Color color,StudioPalette old){
  if(color.ToArgb()==old.Surface.ToArgb())return Surface;if(color.ToArgb()==old.Sidebar.ToArgb())return Sidebar;
  if(color.ToArgb()==old.Background.ToArgb())return Background;if(color.ToArgb()==old.Soft.ToArgb())return Soft;
  if(color.ToArgb()==old.Hover.ToArgb())return Hover;if(color.ToArgb()==old.Border.ToArgb())return Border;return color;
 }
 static Color ForegroundColor(Color color,StudioPalette old){
  if(color.ToArgb()==old.Ink.ToArgb())return Ink;if(color.ToArgb()==old.Muted.ToArgb())return Muted;
  if(color.ToArgb()==old.Accent.ToArgb())return Accent;if(color.ToArgb()==old.OnAccent.ToArgb())return OnAccent;return color;
 }
 static void Apply(Control control,StudioPalette old){
  control.BackColor=BackgroundColor(control.BackColor,old);control.ForeColor=ForegroundColor(control.ForeColor,old);
  var button=control as Button;if(button!=null){button.FlatAppearance.MouseOverBackColor=Hover;button.FlatAppearance.BorderColor=Border;}
  foreach(Control child in control.Controls)Apply(child,old);control.Invalidate();
  if(control is Form)NativeTheme(control);
 }
}

public sealed class StudioThemeWatcher:IDisposable {
 readonly Control dispatcher;
 bool disposed;
 public StudioThemeWatcher(Control owner){dispatcher=owner;SystemEvents.UserPreferenceChanged+=Changed;}
 void Changed(object sender,UserPreferenceChangedEventArgs args){
  if(disposed||StudioColors.Mode!="system"||dispatcher.IsDisposed||!dispatcher.IsHandleCreated)return;
  try{dispatcher.BeginInvoke((Action)delegate{if(!disposed&&!dispatcher.IsDisposed)StudioColors.RefreshSystem();});}catch(InvalidOperationException){}
 }
 public void Dispose(){if(disposed)return;disposed=true;SystemEvents.UserPreferenceChanged-=Changed;}
}
