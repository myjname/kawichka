using System;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Windows.Forms;

public static class PetMenuUiTests {
 static readonly BindingFlags Private=BindingFlags.Instance|BindingFlags.NonPublic;
 static int passed;
 static T Get<T>(object owner,string name){return (T)owner.GetType().GetField(name,Private).GetValue(owner);}
 static void Call(object owner,string name){owner.GetType().GetMethod(name,Private).Invoke(owner,null);}
 static void Check(bool value,string name){if(!value)throw new Exception(name);Console.WriteLine("PASS "+name);passed++;}
 [STAThread] public static int Main(string[] args){
  int at=Array.IndexOf(args,"--data-dir");if(at<0||!args.Contains("--self-test"))return 2;
  Application.SetUnhandledExceptionMode(UnhandledExceptionMode.ThrowException);Application.EnableVisualStyles();Application.SetCompatibleTextRenderingDefault(false);
  try{
   Directory.CreateDirectory(args[at+1]);
   foreach(string language in new[]{"ru","en"}){
    L.Language=language;var form=new MainForm();var pet=new PetForm();ContextMenuStrip menu=null;
    try{
     var settings=Get<Settings>(form,"settings");settings.UiLanguage=language;settings.Voice=false;settings.NotificationSound=false;settings.Paused=false;
     typeof(MainForm).GetField("pet",Private).SetValue(form,pet);pet.Show();
     Call(form,"ShowPetMenu");Application.DoEvents();menu=Get<ContextMenuStrip>(form,"petMenu");
     Call(form,"ShowPetMenu");Application.DoEvents();
     Check(object.ReferenceEquals(menu,Get<ContextMenuStrip>(form,"petMenu"))&&menu.Visible&&!menu.IsDisposed,"double mascot click keeps one live menu "+language);
     menu.Close(ToolStripDropDownCloseReason.AppClicked);Application.DoEvents();
     Check(!menu.IsDisposed,"closing the menu does not dispose it during WinForms dismissal "+language);
     for(int i=0;i<100;i++){Call(form,"ShowPetMenu");menu.Close(ToolStripDropDownCloseReason.AppClicked);Application.DoEvents();}
     Check(!menu.IsDisposed,"100 rapid menu reopenings complete without a disposed handle "+language);
     settings.Paused=true;Call(form,"ShowPetMenu");
     Check(menu.Items.Cast<ToolStripItem>().Any(item=>item.Text==L.T("Освободить Кавычку","Release Kavychka")),"reopening refreshes actions from current settings "+language);
     var pauses=menu.Items.OfType<ToolStripMenuItem>().Single(item=>item.Text==L.T("Пауза","Pause"));pauses.ShowDropDown();Application.DoEvents();menu.Close();Application.DoEvents();Call(form,"ShowPetMenu");
     Check(!menu.IsDisposed&&menu.Visible,"closing an open submenu allows the main menu to reopen "+language);
     var openSettings=menu.Items[0];menu.Close(ToolStripDropDownCloseReason.ItemClicked);openSettings.PerformClick();Application.DoEvents();
     Check(Get<SimpleSettingsForm>(form,"simpleSettings").Visible,"the settings action still works after menu dismissal "+language);
    }finally{
     var settingsForm=Get<SimpleSettingsForm>(form,"simpleSettings");if(settingsForm!=null)settingsForm.Dispose();
     Get<NotifyIcon>(form,"tray").Dispose();Get<Timer>(form,"timer").Dispose();Call(form,"CloseLocalAi");form.Dispose();pet.Dispose();
    }
    Check(menu!=null&&menu.IsDisposed,"disposing the app releases its persistent menu "+language);
   }
   Console.WriteLine("PASS "+passed+" mascot menu checks");return 0;
  }catch(Exception error){Console.Error.WriteLine(error);return 1;}
 }
}
