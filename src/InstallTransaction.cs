using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.IO.Compression;
using System.Linq;
using System.Security.Cryptography;
using System.Text;
using System.Web.Script.Serialization;

public sealed class PackageFile {public string path,sha256;public long bytes;}
public sealed class PackageManifest {public int format;public string version;public PackageFile[] files;}
public sealed class InstallSnapshot {public string path;public bool existed;}
public sealed class InstallJournal {
 public string target,data,workspace,state;
 public InstallSnapshot[] program,preferences;
}

// Shared by the setup executable and isolated transaction tests. App binaries
// are owned by the package; model weights and user recordings never enter it.
public static class InstallTransaction {
 static readonly JavaScriptSerializer Json=new JavaScriptSerializer{MaxJsonLength=4000000};
 const string JournalName=".kavychka-install.json";
 static readonly string[] MutableFiles={"settings.json","settings.json.before-v2","reminder-state.json","library.sqlite3","library.sqlite3-wal","library.sqlite3-shm","legacy-personalization.json","migration-complete.json","migration-pending.json","migration-warnings.json","runtime-ok.json"};
 static string Normal(string path){return Path.GetFullPath(path).TrimEnd(Path.DirectorySeparatorChar,Path.AltDirectorySeparatorChar);}
 static bool Same(string left,string right){return string.Equals(Normal(left),Normal(right),StringComparison.OrdinalIgnoreCase);}
 static string Inside(string root,string relative){
  if(string.IsNullOrWhiteSpace(relative)||Path.IsPathRooted(relative)||relative.IndexOf(':')>=0)throw new InvalidDataException(AppText.T("Неверный путь пакета","Invalid package path"));
  string path=Path.GetFullPath(Path.Combine(root,relative.Replace('/',Path.DirectorySeparatorChar)));
  if(!path.StartsWith(Normal(root)+Path.DirectorySeparatorChar,StringComparison.OrdinalIgnoreCase))throw new InvalidDataException(AppText.T("Путь выходит за выбранную папку","Path escapes the selected folder"));
  string current=path;while(!Same(current,root)){if((Directory.Exists(current)||File.Exists(current))&&(File.GetAttributes(current)&FileAttributes.ReparsePoint)!=0)throw new IOException(AppText.T("Установщик не поддерживает файлы и папки-ссылки","Linked files and folders are not supported by setup"));current=Path.GetDirectoryName(current);}
  if(Directory.Exists(root)&&(File.GetAttributes(root)&FileAttributes.ReparsePoint)!=0)throw new IOException(AppText.T("Папка установки не может быть ссылкой","Linked installation folders are not supported"));return path;
 }
 static bool Owned(string path){return path=="Kavychka.exe"||path=="README.md"||path=="package.json"||
  (path.StartsWith("backend/",StringComparison.Ordinal)&&new[]{".py",".txt",".json"}.Contains(Path.GetExtension(path))&&!path.Split('/').Contains("__pycache__"))||
  new[]{"assets/character.json","assets/components.json","assets/voice/profile.json","assets/voice/default-en.wav"}.Contains(path);}
 static string Digest(string path){using(var hash=SHA256.Create())using(var stream=File.OpenRead(path))return BitConverter.ToString(hash.ComputeHash(stream)).Replace("-","").ToLowerInvariant();}
 static void AtomicJson(string path,object value){string temp=path+".tmp";File.WriteAllText(temp,Json.Serialize(value),new UTF8Encoding(false));if(File.Exists(path))File.Replace(temp,path,null);else File.Move(temp,path);}
 static PackageManifest Manifest(string text){
  var manifest=Json.Deserialize<PackageManifest>(text);Version version;
  if(manifest==null||manifest.format!=1||!Version.TryParse(manifest.version,out version)||manifest.files==null||manifest.files.Length<4||manifest.files.Length>1000)throw new InvalidDataException(AppText.T("Неверный манифест приложения","Invalid application manifest"));
  var names=new HashSet<string>(StringComparer.OrdinalIgnoreCase);
  foreach(var file in manifest.files){if(file==null||!Owned(file.path)||file.path=="package.json"||!names.Add(file.path)||file.path.Contains("\\")||file.path.Split('/').Any(p=>p=="."||p==".."||p.Length==0)||file.bytes<0||file.bytes>50000000||file.sha256==null||file.sha256.Length!=64||file.sha256.Any(c=>!Uri.IsHexDigit(c)))throw new InvalidDataException(AppText.T("Неверная запись манифеста","Invalid manifest entry"));}
  foreach(string required in new[]{"Kavychka.exe","backend/server.py","backend/health_check.py","assets/character.json","assets/voice/profile.json"})if(!names.Contains(required))throw new InvalidDataException(AppText.T("Неполный пакет приложения","Incomplete application package"));
  if(manifest.files.Sum(f=>f.bytes)>200000000)throw new InvalidDataException(AppText.T("Пакет приложения слишком велик","Application package is too large"));return manifest;
 }
 static PackageManifest Extract(Stream payload,string stage){
  using(var zip=new ZipArchive(payload,ZipArchiveMode.Read,true)){
   if(zip.Entries.Count>1100)throw new InvalidDataException(AppText.T("В пакете слишком много файлов","Too many package entries"));
   var manifests=zip.Entries.Where(e=>e.FullName=="package.json").ToArray();if(manifests.Length!=1||manifests[0].Length>4000000)throw new InvalidDataException(AppText.T("Манифест пакета отсутствует","Missing package manifest"));
   string text;using(var reader=new StreamReader(manifests[0].Open(),Encoding.UTF8))text=reader.ReadToEnd();var manifest=Manifest(text);
   var expected=manifest.files.ToDictionary(f=>f.path,StringComparer.OrdinalIgnoreCase);var seen=new HashSet<string>(StringComparer.OrdinalIgnoreCase);
   Directory.CreateDirectory(stage);
   foreach(var entry in zip.Entries){if(entry.Name.Length==0){Inside(stage,entry.FullName);continue;}if(entry.FullName=="package.json")continue;
    PackageFile file;if(!expected.TryGetValue(entry.FullName,out file)||!seen.Add(entry.FullName)||entry.Length!=file.bytes)throw new InvalidDataException(AppText.T("Неожиданное содержимое пакета","Unexpected package content"));
    string target=Inside(stage,file.path);Directory.CreateDirectory(Path.GetDirectoryName(target));entry.ExtractToFile(target);if(!Digest(target).Equals(file.sha256,StringComparison.OrdinalIgnoreCase))throw new InvalidDataException(AppText.T("Проверка целостности файла приложения не пройдена: ","Application file integrity check failed: ")+file.path);
   }
   if(seen.Count!=expected.Count)throw new InvalidDataException(AppText.T("Файл приложения отсутствует","An application file is missing"));File.WriteAllText(Inside(stage,"package.json"),text,new UTF8Encoding(false));return manifest;
  }
 }
 static InstallSnapshot[] Backup(string root,string backup,IEnumerable<string> paths){
  Directory.CreateDirectory(backup);var result=new List<InstallSnapshot>();
  foreach(string name in paths.Distinct(StringComparer.OrdinalIgnoreCase)){string source=Inside(root,name),target=Inside(backup,name);bool exists=File.Exists(source);if(Directory.Exists(source))throw new IOException(AppText.T("Вместо файла приложения найдена папка: ","An application file is occupied by a folder: ")+name);if(exists){Directory.CreateDirectory(Path.GetDirectoryName(target));File.Copy(source,target);}result.Add(new InstallSnapshot{path=name,existed=exists});}return result.ToArray();
 }
 static void Restore(string root,string backup,InstallSnapshot[] files,bool program){
  foreach(var entry in files){if(program?!Owned(entry.path):!MutableFiles.Contains(entry.path))throw new InvalidDataException(AppText.T("Неверная запись отката","Invalid rollback entry"));string target=Inside(root,entry.path);if(entry.existed){string source=Inside(backup,entry.path);if(!File.Exists(source))throw new IOException(AppText.T("Не найден файл резервной копии: ","Rollback file is missing: ")+entry.path);if(File.Exists(target)&&Digest(source)==Digest(target))continue;Directory.CreateDirectory(Path.GetDirectoryName(target));File.Copy(source,target,true);}else if(File.Exists(target))File.Delete(target);}
 }
 static void ValidateWorkspace(string workspace,string target){
  if(!Same(Path.GetDirectoryName(Normal(workspace)),Path.GetDirectoryName(Normal(target)))||!Path.GetFileName(workspace).StartsWith(".kavychka-setup-",StringComparison.Ordinal))throw new InvalidDataException(AppText.T("Неверная рабочая папка установщика","Invalid setup workspace"));
  Inside(Path.GetDirectoryName(Normal(target)),Path.GetFileName(workspace));
 }
 static void RemoveWorkspace(string workspace,string target){
  // Resolve and verify the complete tree before a recursive removal on Windows.
  ValidateWorkspace(workspace,target);if(!Directory.Exists(workspace))return;CheckTree(workspace,workspace);Directory.Delete(workspace,true);
 }
 static void CheckTree(string root,string folder){foreach(string item in Directory.GetFileSystemEntries(folder)){Inside(root,item.Substring(Normal(root).Length+1));if(Directory.Exists(item))CheckTree(root,item);}}
 static void KeepRecentBackups(string target){
  var owned=new List<string>();foreach(string directory in Directory.GetDirectories(Path.GetDirectoryName(target),".kavychka-setup-*")){
   try{ValidateWorkspace(directory,target);string marker=Inside(directory,"owner.json");if(!File.Exists(marker))continue;var item=Json.Deserialize<Dictionary<string,object>>(File.ReadAllText(marker));if(item.ContainsKey("committed")&&Convert.ToBoolean(item["committed"])&&Same(Convert.ToString(item["target"]),target))owned.Add(directory);}catch(IOException){}catch(ArgumentException){}
  }
  foreach(string old in owned.OrderByDescending(Directory.GetLastWriteTimeUtc).Skip(2))try{RemoveWorkspace(old,target);}catch(IOException){}catch(UnauthorizedAccessException){}
 }
 public static void Recover(string target,string data,Action<string> progress){
  string file=Inside(target,JournalName);if(!File.Exists(file))return;var journal=Json.Deserialize<InstallJournal>(File.ReadAllText(file));
  if(journal==null||!Same(journal.target,target)||!Same(journal.data,data))throw new InvalidDataException(AppText.T("Незавершённая установка принадлежит другой папке данных","Unfinished setup belongs to another data folder"));ValidateWorkspace(journal.workspace,target);
  if(journal.state!="committed"){progress(AppText.T("Восстанавливаю предыдущую версию","Restoring the previous version"));Restore(target,Path.Combine(journal.workspace,"program"),journal.program,true);Restore(data,Path.Combine(journal.workspace,"preferences"),journal.preferences,false);}
  File.Delete(file);if(journal.state!="committed")RemoveWorkspace(journal.workspace,target);
 }
 public static void Apply(Stream payload,string target,string data,Action<string> progress,Action<string,string,string> healthCheck){
  target=Normal(target);data=Normal(data);if(Same(target,Path.GetPathRoot(target))||Same(target,data))throw new ArgumentException(AppText.T("Выберите отдельную папку приложения","Choose a separate application folder"));
  Directory.CreateDirectory(target);Directory.CreateDirectory(data);Inside(target,JournalName);Inside(data,"settings.json");Recover(target,data,progress);
  string workspace=Path.Combine(Path.GetDirectoryName(target),".kavychka-setup-"+Guid.NewGuid().ToString("N"));ValidateWorkspace(workspace,target);Directory.CreateDirectory(workspace);
  bool prepared=false,committed=false;
  try{
   progress(AppText.T("Проверяю пакет","Verifying the package"));string stage=Inside(workspace,"stage");var manifest=Extract(payload,stage);
   progress(AppText.T("Сохраняю настройки и пресеты","Backing up settings and presets"));LegacyPersonalization.Capture(target,data);
   var owned=manifest.files.Select(f=>f.path).Concat(new[]{"package.json"}).ToList();string oldManifest=Inside(target,"package.json");if(File.Exists(oldManifest))owned.AddRange(Manifest(File.ReadAllText(oldManifest)).files.Select(f=>f.path));
   var journal=new InstallJournal{target=target,data=data,workspace=workspace,state="prepared",program=Backup(target,Inside(workspace,"program"),owned),preferences=Backup(data,Inside(workspace,"preferences"),MutableFiles)};
   AtomicJson(Inside(workspace,"owner.json"),new{target=target,version=manifest.version,created_utc=DateTime.UtcNow.ToString("o")});AtomicJson(Inside(target,JournalName),journal);prepared=true;
   progress(AppText.T("Обновляю файлы","Updating app files"));
   foreach(var file in journal.program){string source=Inside(stage,file.path),destination=Inside(target,file.path);if(File.Exists(source)){Directory.CreateDirectory(Path.GetDirectoryName(destination));File.Copy(source,destination,true);}else if(File.Exists(destination))File.Delete(destination);}
   progress(AppText.T("Проверяю запуск и миграцию","Checking startup and migration"));healthCheck(target,data,manifest.version);
   journal.state="committed";AtomicJson(Inside(target,JournalName),journal);committed=true;File.Delete(Inside(target,JournalName));
   // Cleanup cannot turn a verified, committed update into a reported failure.
   try{AtomicJson(Inside(workspace,"owner.json"),new{target=target,version=manifest.version,committed=true,created_utc=DateTime.UtcNow.ToString("o")});string stageRoot=Inside(workspace,"stage");CheckTree(stageRoot,stageRoot);Directory.Delete(stageRoot,true);KeepRecentBackups(target);}catch(IOException){}catch(UnauthorizedAccessException){}
   progress(AppText.T("Готово","Ready"));
  }catch(Exception failure){
   if(prepared&&!committed){Recover(target,data,progress);prepared=false;throw new IOException(AppText.T("Обновление отменено: предыдущая программа и настройки восстановлены.","Update cancelled: the previous application and preferences were restored."),failure);}
   throw;
  }finally{if(!prepared&&!committed&&Directory.Exists(workspace))RemoveWorkspace(workspace,target);}
 }
 static string Quote(string value){if(value.Contains("\""))throw new ArgumentException(AppText.T("Неверный путь","Invalid path"));return "\""+value+"\"";}
 public static void CheckStartup(string target,string data,string version){
  string report=Path.Combine(target,".kavychka-health.json");if(File.Exists(report))File.Delete(report);
  var info=new ProcessStartInfo(Path.Combine(target,"Kavychka.exe"),"--health-check "+Quote(report)+" --data-dir "+Quote(data)){WorkingDirectory=target,UseShellExecute=false,CreateNoWindow=true,WindowStyle=ProcessWindowStyle.Hidden};
  using(var process=Process.Start(info)){if(!process.WaitForExit(60000)){process.Kill();process.WaitForExit();throw new IOException(AppText.T("Проверка запуска превысила время ожидания","Startup check timed out"));}if(process.ExitCode!=0||!File.Exists(report))throw new IOException(AppText.T("Новая версия не смогла запуститься. Предыдущая версия будет восстановлена.","The new version could not start. The previous version will be restored."));}
  var result=Json.Deserialize<Dictionary<string,object>>(File.ReadAllText(report));if(!Convert.ToBoolean(result["ok"])||Convert.ToString(result["version"])!=version)throw new IOException(AppText.T("Неожиданный результат проверки запуска","Unexpected startup verification result"));File.Delete(report);
 }
}
