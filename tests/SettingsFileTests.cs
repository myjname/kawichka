using System;
using System.Diagnostics;
using System.IO;

public static class SettingsFileTests {
 static int passed;
 static void Check(bool yes,string name){if(!yes)throw new Exception(name);Console.WriteLine("PASS "+name);passed++;}
 public static int Main(){
  string work=Path.GetFullPath(Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"..","..","work")),root=Path.Combine(work,"settings-tests-"+Guid.NewGuid().ToString("N"));
  try{
   Directory.CreateDirectory(root);string file=Path.Combine(root,"settings.json");SettingsFile.Write(file,"{\"name\":\"Кавычка\"}");Check(File.ReadAllText(file)=="{\"name\":\"Кавычка\"}","first settings save preserves Unicode");
   SettingsFile.Write(file,"second version");Check(File.ReadAllText(file)=="second version","an existing settings file is replaced atomically");
   bool failed=false;using(var locked=new FileStream(file,FileMode.Open,FileAccess.ReadWrite,FileShare.None)){try{SettingsFile.Write(file,"must not replace");}catch(IOException){failed=true;}}
   Check(failed&&File.ReadAllText(file)=="second version","a failed save preserves the previous settings");
   string occupied=Path.Combine(root,"occupied.json");Directory.CreateDirectory(occupied);failed=false;try{SettingsFile.Write(occupied,"new settings");}catch(IOException){failed=true;}
   Check(failed&&Directory.Exists(occupied),"a settings path occupied by a folder reports failure");
   Check(Directory.GetFiles(root,"*.tmp-*").Length==0,"failed saves clean up their own temporary files");
   string archive=Path.Combine(root,"archive.zip");SettingsFile.WriteBytes(archive,new byte[]{1,2,3});failed=false;using(var locked=new FileStream(archive,FileMode.Open,FileAccess.Read,FileShare.Read)){try{SettingsFile.WriteBytes(archive,new byte[]{9,8,7});}catch(IOException){failed=true;}}
   Check(failed&&Convert.ToBase64String(File.ReadAllBytes(archive))==Convert.ToBase64String(new byte[]{1,2,3}),"a failed archive replacement preserves the previous export");
   string data=Path.Combine(root,"health");Directory.CreateDirectory(data);string preferences=Path.Combine(data,"settings.json"),report=Path.Combine(data,"report.json");File.WriteAllText(preferences,"{\"SetupComplete\":true,\"VoiceProvider\":\"System\"}");byte[] original=File.ReadAllBytes(preferences);
   using(var locked=new FileStream(preferences,FileMode.Open,FileAccess.Read,FileShare.Read)){
    var info=new ProcessStartInfo(Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"Kavychka.exe"),"--health-check \""+report+"\" --data-dir \""+data+"\""){UseShellExecute=false,CreateNoWindow=true,WindowStyle=ProcessWindowStyle.Hidden};
    using(var process=Process.Start(info)){if(!process.WaitForExit(30000)){process.Kill();process.WaitForExit();throw new TimeoutException("Health check timed out");}Check(process.ExitCode!=0&&!File.Exists(report),"startup verification fails if preferences cannot be saved");}
   }
   Check(Convert.ToBase64String(original)==Convert.ToBase64String(File.ReadAllBytes(preferences)),"failed startup verification preserves locked preferences byte for byte");
   Console.WriteLine("PASS "+passed+" persistence checks");return 0;
  }catch(Exception error){Console.Error.WriteLine(error);return 1;}
  finally{if(root.StartsWith(work+Path.DirectorySeparatorChar,StringComparison.OrdinalIgnoreCase)&&Path.GetFileName(root).StartsWith("settings-tests-",StringComparison.Ordinal)&&Directory.Exists(root))Directory.Delete(root,true);}
 }
}
