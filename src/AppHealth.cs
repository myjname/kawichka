using System;
using System.Diagnostics;
using System.IO;
using System.Windows.Forms;

public partial class MainForm {
 static void CheckAppHealth(string report){
  Application.EnableVisualStyles();Application.SetCompatibleTextRenderingDefault(false);
  using(var form=new MainForm()){
   try{
    form.timer.Stop();form.dailyTimer.Stop();form.tray.Visible=false;
    if(!string.IsNullOrEmpty(form.settingsSaveError))throw new IOException(form.settingsSaveError);
    RitualSchedule.Validate(form.settings);
    using(var simple=new SimpleSettingsForm(form.settings,PersonalPresets.List(Folder),false)){simple.CreateControl();}
    string python=form.PythonPath();if(File.Exists(python)&&File.Exists(Path.Combine(Folder,"library.sqlite3"))){
     var info=new ProcessStartInfo(python,"-B "+MediaTools.Q(Path.Combine(Application.StartupPath,"backend","health_check.py"))+" --data "+MediaTools.Q(Folder)){UseShellExecute=false,CreateNoWindow=true,RedirectStandardOutput=true,RedirectStandardError=true,WorkingDirectory=Application.StartupPath};
     using(var process=Process.Start(info)){var output=process.StandardOutput.ReadToEndAsync();var error=process.StandardError.ReadToEndAsync();if(!process.WaitForExit(45000)){process.Kill();process.WaitForExit();throw new IOException(AppText.T("Перенос библиотеки превысил время ожидания","Library migration timed out"));}output.GetAwaiter().GetResult();string details=error.GetAwaiter().GetResult();if(process.ExitCode!=0)throw new IOException(AppText.T("Не удалось перенести библиотеку: ","Library migration failed: ")+details);}
    }
    File.WriteAllText(report,Json.Serialize(new{ok=true,version=AppUpdates.Current.ToString(),data=Folder}));
   }finally{form.exiting=true;form.tray.Dispose();form.timer.Dispose();form.dailyTimer.Dispose();form.Dispose();}
  }
 }
}
