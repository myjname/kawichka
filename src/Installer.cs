using System;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Threading;
using System.Threading.Tasks;
using System.Windows.Forms;

public sealed class Installer:Form {
 static bool Russian=System.Globalization.CultureInfo.CurrentUICulture.TwoLetterISOLanguageName=="ru";
 static string T(string ru,string en){return Russian?ru:en;}
 readonly string[] args;
 TextBox path;
 Label status;
 Button install;
 bool running;
 static string Value(string[] args,string flag,string fallback){int i=Array.IndexOf(args,flag);if(i<0)return fallback;if(i+1>=args.Length)throw new ArgumentException(AppText.T("Не задано значение для ","Missing value for ")+flag);return args[i+1];}
 static string DefaultTarget{get{return Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),"Programs","Kavychka");}}
 [STAThread] public static int Main(string[] args){
  try{
   Russian=Value(args,"--language",Russian?"ru":"en")=="ru";
   AppText.Language=Russian?"ru":"en";
   if(args.Contains("--silent")){string target=Value(args,"--target",null);if(target==null)throw new ArgumentException(AppText.T("Нужен параметр --target","--target is required"));Run(args,target,delegate{});return 0;}
   Application.EnableVisualStyles();Application.Run(new Installer(args));return Environment.ExitCode;
  }catch(Exception error){Environment.ExitCode=1;string log=Path.Combine(Path.GetTempPath(),"Kavychka-setup-error.log");File.WriteAllText(log,error.ToString());if(!args.Contains("--silent"))MessageBox.Show(T("Установка не завершена. ","Setup did not complete. ")+error.Message+"\n"+log,"Kavychka",MessageBoxButtons.OK,MessageBoxIcon.Error);return 1;}
 }
 Installer(string[] arguments){
  args=arguments;bool update=args.Contains("--update");
  Text=update?T("Обновление Кавычки","Update Kavychka"):T("Установка Кавычки","Install Kavychka");ClientSize=new Size(600,350);StartPosition=FormStartPosition.CenterScreen;BackColor=Color.FromArgb(248,249,252);ForeColor=Color.FromArgb(35,40,61);Font=new Font("Segoe UI",10);MaximizeBox=false;FormBorderStyle=FormBorderStyle.FixedDialog;
  Controls.Add(new Label{Text="❝  "+T("Кавычка","Kavychka"),Font=new Font("Segoe UI",24,FontStyle.Bold),Location=new Point(24,22),Size=new Size(550,51)});
  Controls.Add(new Label{Text=T("Напоминания работают сразу. Локальные модели для новых фраз и голосов можно подключить позже.","Reminders work immediately. Local text and voice models can be added later."),Location=new Point(28,87),Size=new Size(540,55),ForeColor=Color.FromArgb(112,118,139)});
  path=new TextBox{Text=Value(args,"--target",DefaultTarget),Location=new Point(28,153),Width=540,ReadOnly=update};Controls.Add(path);
  install=new Button{Text=update?T("Обновить и открыть","Update and open"):T("Установить и открыть","Install and open"),Location=new Point(28,203),Size=new Size(260,42),FlatStyle=FlatStyle.Flat,BackColor=Color.FromArgb(112,91,225),ForeColor=Color.White};install.FlatAppearance.BorderSize=0;Controls.Add(install);
  status=new Label{Text=T("Модели, записи и настройки сохраняются отдельно от приложения и подхватываются после обновления.","Models, recordings and settings are stored separately and reused after updates."),Location=new Point(28,269),Size=new Size(540,67),ForeColor=Color.FromArgb(112,118,139)};Controls.Add(status);
  install.Click+=async delegate{await Install();};if(update)Shown+=async delegate{await Install();};
  FormClosing+=delegate(object sender,FormClosingEventArgs e){if(running)e.Cancel=true;};
 }
 async Task Install(){if(running)return;running=true;install.Enabled=false;path.Enabled=false;
  try{string target=Path.GetFullPath(path.Text);var progress=new Progress<string>(message=>{if(!IsDisposed)status.Text=message;});await Task.Run(delegate{Run(args,target,message=>((IProgress<string>)progress).Report(message));});running=false;Process.Start(new ProcessStartInfo(Path.Combine(target,"Kavychka.exe")){WorkingDirectory=target,UseShellExecute=true});Close();}
  catch(Exception error){Environment.ExitCode=1;status.Text=T("Не удалось завершить установку: ","Setup failed: ")+error.Message;running=false;install.Enabled=true;path.Enabled=true;}
 }
 static void Run(string[] args,string target,Action<string> progress){
  target=Path.GetFullPath(target);string pid=Value(args,"--wait-pid",null);
  if(pid!=null){int id;if(!int.TryParse(pid,out id)||id<=0)throw new ArgumentException(AppText.T("Неверный идентификатор процесса","Invalid process ID"));
   Process old=null;try{old=Process.GetProcessById(id);}catch(ArgumentException){}
   if(old!=null)using(old){if(!old.HasExited){if(!string.Equals(Path.GetFullPath(old.MainModule.FileName),Path.Combine(target,"Kavychka.exe"),StringComparison.OrdinalIgnoreCase))throw new IOException(AppText.T("Ожидаемый процесс не принадлежит этой установке","The waiting process is not this installation"));progress(T("Жду завершения приложения…","Waiting for the app to exit…"));if(!old.WaitForExit(60000))throw new IOException(T("Закрой Кавычку и повтори обновление.","Close Kavychka and retry the update."));}}
  }
  bool owned;using(var gate=new Mutex(true,"Local\\KavychkaBreaks-v1",out owned)){if(!owned)throw new IOException(T("Кавычка запущена. Закрой её через значок у часов и повтори установку.","Kavychka is running. Close it using the tray icon and retry setup."));try{
   string data=SharedStorage.Resolve(target,Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),args);
   using(var payload=Assembly.GetExecutingAssembly().GetManifestResourceStream("payload.zip")){if(payload==null)throw new IOException(AppText.T("Пакет приложения отсутствует","Missing application payload"));InstallTransaction.Apply(payload,target,data,progress,InstallTransaction.CheckStartup);}
  }finally{gate.ReleaseMutex();}}
 }
}
