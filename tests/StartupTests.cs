using System;
using System.IO;

public static class StartupTests {
 static int passed;
 static void Check(bool yes,string name){if(!yes)throw new Exception(name);Console.WriteLine("PASS "+name);passed++;}
 [STAThread] public static int Main(){
  string work=Path.GetFullPath(Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"..","..","work"));string root=Path.Combine(work,"startup-tests-"+Guid.NewGuid().ToString("N"));
  try{
   string startup=Path.Combine(root,"Startup"),app=Path.Combine(root,"App with spaces","Kavychka.exe");Directory.CreateDirectory(Path.GetDirectoryName(app));File.WriteAllText(app,"not executed");
   Check(!StartupIntegration.IsEnabled(startup)&&StartupIntegration.Read(startup)==null,"new installation has no implicit startup entry");
   StartupIntegration.SetEnabled(startup,app,true);var entry=StartupIntegration.Read(startup);
   Check(StartupIntegration.IsEnabled(startup)&&string.Equals(entry.Target,app,StringComparison.OrdinalIgnoreCase),"real Windows shortcut targets the selected application");
   Check(entry.Arguments=="--tray"&&string.Equals(entry.WorkingDirectory,Path.GetDirectoryName(app),StringComparison.OrdinalIgnoreCase),"startup uses mascot mode and the correct working folder");
   StartupIntegration.SetEnabled(startup,app,true);Check(Directory.GetFiles(startup).Length==1,"enabling startup twice is idempotent");
   string updated=Path.Combine(root,"Updated app","Kavychka.exe");Directory.CreateDirectory(Path.GetDirectoryName(updated));File.WriteAllText(updated,"new executable");StartupIntegration.SetEnabled(startup,updated,true);
   Check(string.Equals(StartupIntegration.Read(startup).Target,updated,StringComparison.OrdinalIgnoreCase),"startup can follow a new installation folder");
   bool rejected=false;try{StartupIntegration.SetEnabled(startup,Path.Combine(root,"missing.exe"),true);}catch(FileNotFoundException){rejected=true;}
   Check(rejected&&string.Equals(StartupIntegration.Read(startup).Target,updated,StringComparison.OrdinalIgnoreCase),"failed startup setup preserves the working shortcut");
   string other=Path.Combine(startup,"Other app.lnk");File.WriteAllText(other,"unrelated entry");StartupIntegration.SetEnabled(startup,updated,false);
   Check(!StartupIntegration.IsEnabled(startup)&&File.ReadAllText(other)=="unrelated entry","disabling removes only Kavychka's shortcut");
   StartupIntegration.SetEnabled(startup,updated,false);Check(Directory.GetFiles(startup).Length==1,"disabling an absent shortcut is harmless");
   Check(Directory.GetFiles(startup,".KavychkaBreaks-*").Length==0,"startup changes leave no temporary shortcuts");
   Console.WriteLine("PASS "+passed+" startup checks");return 0;
  }catch(Exception error){Console.Error.WriteLine(error);return 1;}
  finally{if(root.StartsWith(work+Path.DirectorySeparatorChar,StringComparison.OrdinalIgnoreCase)&&Path.GetFileName(root).StartsWith("startup-tests-",StringComparison.Ordinal)&&Directory.Exists(root))Directory.Delete(root,true);}
 }
}
