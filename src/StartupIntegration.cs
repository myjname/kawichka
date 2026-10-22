using System;
using System.IO;
using System.Runtime.InteropServices;

public sealed class StartupEntry {
 public string Target,Arguments,WorkingDirectory;
}

// A per-user shortcut needs no administrator rights. The folder argument also
// allows verification with real Windows shortcuts in an isolated test folder.
public static class StartupIntegration {
 public static string Folder {get{return Environment.GetFolderPath(Environment.SpecialFolder.Startup);}}
 static string FilePath(string folder){return Path.Combine(Path.GetFullPath(folder),"KavychkaBreaks.lnk");}
 public static bool IsEnabled(string folder){return File.Exists(FilePath(folder));}
 static void Release(object value){if(value!=null&&Marshal.IsComObject(value))Marshal.FinalReleaseComObject(value);}
 public static StartupEntry Read(string folder){
  string path=FilePath(folder);if(!File.Exists(path))return null;
  object shell=null,shortcut=null;
  try{shell=Activator.CreateInstance(Type.GetTypeFromProgID("WScript.Shell"));shortcut=((dynamic)shell).CreateShortcut(path);return new StartupEntry{Target=((dynamic)shortcut).TargetPath,Arguments=((dynamic)shortcut).Arguments,WorkingDirectory=((dynamic)shortcut).WorkingDirectory};}
  finally{Release(shortcut);Release(shell);}
 }
 public static void SetEnabled(string folder,string executable,bool enabled){
  string path=FilePath(folder);
  if(File.Exists(path)&&(File.GetAttributes(path)&FileAttributes.ReparsePoint)!=0)throw new IOException(AppText.T("Ярлык автозапуска является ссылкой","The startup shortcut is a linked file"));
  if(!enabled){if(File.Exists(path))File.Delete(path);return;}
  executable=Path.GetFullPath(executable);if(!File.Exists(executable))throw new FileNotFoundException(AppText.T("Файл приложения отсутствует","Application executable is missing"),executable);
  Directory.CreateDirectory(Path.GetDirectoryName(path));string temp=Path.Combine(Path.GetDirectoryName(path),".KavychkaBreaks-"+Guid.NewGuid().ToString("N")+".lnk");
  object shell=null,shortcut=null;
  try{
   shell=Activator.CreateInstance(Type.GetTypeFromProgID("WScript.Shell"));shortcut=((dynamic)shell).CreateShortcut(temp);
   ((dynamic)shortcut).TargetPath=executable;((dynamic)shortcut).Arguments="--tray";((dynamic)shortcut).WorkingDirectory=Path.GetDirectoryName(executable);((dynamic)shortcut).Description="Kavychka";((dynamic)shortcut).Save();
   Release(shortcut);shortcut=null;Release(shell);shell=null;
   if(File.Exists(path))File.Replace(temp,path,null);else File.Move(temp,path);
  }finally{Release(shortcut);Release(shell);if(File.Exists(temp))File.Delete(temp);}
 }
}
