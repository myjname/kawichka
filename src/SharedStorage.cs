using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Security.Cryptography;
using System.Threading;
using System.Threading.Tasks;
using System.Web.Script.Serialization;

public static class SharedStorage {
 static readonly JavaScriptSerializer Json=new JavaScriptSerializer();
 public static bool IsIsolated(string[] args) {return args.Contains("--preview")||args.Contains("--data-dir");}
 public static string Resolve(string app,string local,string[] args) {
  int index=Array.IndexOf(args,"--data-dir");
  if(index>=0) {if(index+1>=args.Length)throw new ArgumentException(AppText.T("После --data-dir нужен путь.","A path is required after --data-dir."));return Path.GetFullPath(args[index+1]);}
  index=Array.IndexOf(args,"--preview");
  if(index>=0) return Path.Combine(Path.GetDirectoryName(Path.GetFullPath(args[index+1])),"preview-data");
  string location=Path.Combine(local,"Kavychka-storage.json");
  if(File.Exists(location)) {
   var saved=Json.Deserialize<Dictionary<string,object>>(File.ReadAllText(location));
   string chosen=Convert.ToString(saved["directory"]);
   if(!Path.IsPathRooted(chosen)||!Directory.Exists(chosen)||!File.Exists(Path.Combine(chosen,"storage-owner.json")))throw new IOException(AppText.T("Папка хранилища недоступна: ","Storage folder unavailable: ")+chosen+AppText.T(". Подключите диск; данные заново не скачиваются.",". Connect the drive; data will not be downloaded again."));
   return Path.GetFullPath(chosen);
  }
  return Path.Combine(local,"Kavychka");
 }
 static IEnumerable<string> RelocationFiles(string folder) {
  if((File.GetAttributes(folder)&FileAttributes.ReparsePoint)!=0)throw new IOException(AppText.T("Перенос папок-ссылок не поддерживается.","Linked folders cannot be moved."));
  foreach(string file in Directory.GetFiles(folder)){if((File.GetAttributes(file)&FileAttributes.ReparsePoint)!=0)throw new IOException(AppText.T("В хранилище обнаружен файл-ссылка.","A linked file was found in storage."));yield return file;}
  foreach(string child in Directory.GetDirectories(folder))foreach(string file in RelocationFiles(child))yield return file;
 }
 public static int Relocate(string source,string target,string local,CancellationToken cancel,Action<string,long,long> progress,Action<string> verify) {
  source=Normal(source);target=Normal(target);
  if(!Directory.Exists(source))throw new IOException(AppText.T("Исходная папка хранилища недоступна: ","The original storage folder is unavailable: ")+source+AppText.T(". Подключите диск и повторите перенос.",". Connect the drive and retry the move."));
  if(Same(source,target)||Nested(source,target)||Nested(target,source)||Same(target,Path.GetPathRoot(target)))throw new IOException(AppText.T("Выберите отдельную пустую папку вне текущего хранилища.","Choose a separate empty folder outside the current storage."));
  if(!Same(source,Path.Combine(local,"Kavychka"))&&!File.Exists(Path.Combine(source,"storage-owner.json")))throw new IOException(AppText.T("Исходная папка не помечена как хранилище Кавычки.","The original folder is not marked as Kavychka storage."));
  if(Directory.Exists(target)&&Directory.EnumerateFileSystemEntries(target).Any())throw new IOException(AppText.T("Для переноса нужна пустая папка. Существующие файлы не перезаписываются.","Moving requires an empty folder. Existing files are never overwritten."));
  if(Directory.Exists(target)&&(File.GetAttributes(target)&FileAttributes.ReparsePoint)!=0)throw new IOException(AppText.T("Выберите обычную папку, не ссылку.","Choose a regular folder rather than a link."));
  var files=RelocationFiles(source).ToArray();
  long total=files.Sum(f=>new FileInfo(f).Length),done=0;
  var drive=new DriveInfo(Path.GetPathRoot(target));if(drive.AvailableFreeSpace<total+100000000)throw new IOException(AppText.T("На выбранном диске недостаточно места для проверенной копии.","The selected drive has insufficient space for a verified copy."));
  cancel.ThrowIfCancellationRequested();Directory.CreateDirectory(target);
  var copied=new List<string>();var stamps=new Dictionary<string,DateTime>();
  bool committed=false;
  try {
   foreach(string file in files) {
    cancel.ThrowIfCancellationRequested();
    string destination=Path.GetFullPath(Path.Combine(target,file.Substring(source.Length+1)));
    if(!Nested(destination,target))throw new IOException(AppText.T("Путь переноса выходит за выбранную папку.","The move path is outside the selected folder."));
    Directory.CreateDirectory(Path.GetDirectoryName(destination));copied.Add(destination);
    stamps[file]=File.GetLastWriteTimeUtc(file);
    byte[] original;
    using(var hash=SHA256.Create())using(var input=File.OpenRead(file))using(var output=new FileStream(destination,FileMode.CreateNew,FileAccess.Write,FileShare.None)) {
     byte[] buffer=new byte[4*1024*1024];int count;
     while((count=input.Read(buffer,0,buffer.Length))>0){cancel.ThrowIfCancellationRequested();output.Write(buffer,0,count);hash.TransformBlock(buffer,0,count,null,0);done+=count;progress(AppText.T("Перенос: ","Moving: ")+Path.GetFileName(file),done,total);}
     hash.TransformFinalBlock(new byte[0],0,0);original=hash.Hash;
    }
    using(var hash=SHA256.Create())using(var input=File.OpenRead(destination))if(!original.SequenceEqual(hash.ComputeHash(input)))throw new IOException(AppText.T("Проверка копии не пройдена: ","Copy verification failed: ")+file);
    if(stamps[file]!=File.GetLastWriteTimeUtc(file))throw new IOException(AppText.T("Исходный файл изменился во время переноса: ","The original file changed during the move: ")+file);
   }
   cancel.ThrowIfCancellationRequested();verify(target);cancel.ThrowIfCancellationRequested();
   string owner=Path.Combine(target,"storage-owner.json");if(!copied.Contains(owner))copied.Add(owner);
   AtomicText(owner,"{\"application\":\"Kavychka\",\"version\":1}");
   string ignore=Path.Combine(target,".gitignore");if(!copied.Contains(ignore))copied.Add(ignore);AtomicText(ignore,"# Personal Kavychka storage\n*\n");
   Directory.CreateDirectory(local);AtomicText(Path.Combine(local,"Kavychka-storage.json"),Json.Serialize(new {directory=target}));committed=true;
  } finally {
   if(!committed){foreach(string file in copied)if(File.Exists(file))File.Delete(file);RemoveEmpty(target,copied);}
  }
  int remaining=0;
  foreach(string file in files) {
   try {if(File.GetLastWriteTimeUtc(file)!=stamps[file]){remaining++;continue;}File.SetAttributes(file,FileAttributes.Normal);File.Delete(file);if(File.Exists(file))remaining++;}catch(IOException){remaining++;}catch(UnauthorizedAccessException){remaining++;}
  }
  // Delete empty directories only; unrelated new files and reparse points are never traversed for deletion.
  remaining+=RemoveEmpty(source,files);
  if(Directory.Exists(source))remaining++;
  progress(AppText.T("Папка хранилища изменена","Storage folder changed"),total,total);return remaining;
 }
 static int RemoveEmpty(string source,IEnumerable<string> files) {
  int failures=0;
  foreach(string directory in files.Select(Path.GetDirectoryName).Distinct().OrderByDescending(p=>p.Length)) {
   string current=directory;
   try {while((Same(current,source)||Nested(current,source))&&Directory.Exists(current)&&!Directory.EnumerateFileSystemEntries(current).Any()) {Directory.Delete(current,false);if(Same(current,source))break;current=Path.GetDirectoryName(current);}}catch(IOException){failures++;}catch(UnauthorizedAccessException){failures++;}
  }
  if(Directory.Exists(source)&&!Directory.EnumerateFileSystemEntries(source).Any())Directory.Delete(source,false);
  return failures;
 }
 static string Normal(string path) {return Path.GetFullPath(path).TrimEnd(Path.DirectorySeparatorChar,Path.AltDirectorySeparatorChar);}
 static bool Same(string a,string b) {return string.Equals(Normal(a),Normal(b),StringComparison.OrdinalIgnoreCase);}
 static bool Nested(string a,string b) {return Normal(a).StartsWith(Normal(b)+Path.DirectorySeparatorChar,StringComparison.OrdinalIgnoreCase);}
 public static string[] Pending(string app,string local,string target) {
  var complete=new HashSet<string>(StringComparer.OrdinalIgnoreCase);
  string journal=Path.Combine(target,"migration-complete.json");
  if(File.Exists(journal))foreach(string path in Json.Deserialize<string[]>(File.ReadAllText(journal)))complete.Add(Normal(path));
  // Components prepared by the packaged Codex process may have been redirected
  // by Windows. Import them into the normal shared store on a standalone launch.
  return new[]{Path.Combine(app,"data"),Path.Combine(local,"Programs","Kavychka","data"),Path.Combine(local,"KavychkaBreaks"),Path.Combine(local,"Packages","OpenAI.Codex_2p2nqsd0c76g0","LocalCache","Local","Kavychka")}
   .Select(Normal).Distinct(StringComparer.OrdinalIgnoreCase)
   .Where(path=>Directory.Exists(path)&&!Same(path,target)&&!Nested(target,path)&&!Nested(path,target)&&!complete.Contains(path)&&
    (File.GetAttributes(path)&FileAttributes.ReparsePoint)==0&&
    (File.Exists(Path.Combine(path,"settings.json"))||File.Exists(Path.Combine(path,"library.sqlite3"))||Directory.Exists(Path.Combine(path,"models"))||Directory.Exists(Path.Combine(path,"runtime"))))
   .ToArray();
 }
 public static void Preferences(string target,IEnumerable<string> sources) {
  string destination=Path.Combine(target,"settings.json");if(File.Exists(destination))return;
  var allowed=new HashSet<string>(new[]{"VoiceProvider","Swear","Voice","NotificationSound","Paused","ExerciseEnabled","LastMeal","LastBreak","ExerciseUtc","History","LocalAiMigrated"});
  foreach(string source in sources) {
   string path=Path.Combine(source,"settings.json");if(!File.Exists(path))continue;
   var old=Json.Deserialize<Dictionary<string,object>>(File.ReadAllText(path));
   var clean=old.Where(item=>allowed.Contains(item.Key)).ToDictionary(item=>item.Key,item=>item.Value);
   clean["VoiceProvider"]="Qwen local";Directory.CreateDirectory(target);AtomicText(destination,Json.Serialize(clean));return;
  }
 }
 static IEnumerable<string> Files(string folder) {
  if(!Directory.Exists(folder)||(File.GetAttributes(folder)&FileAttributes.ReparsePoint)!=0)yield break;
  foreach(string file in Directory.GetFiles(folder))if((File.GetAttributes(file)&FileAttributes.ReparsePoint)==0&&Path.GetExtension(file)!=".pyc"&&Path.GetExtension(file)!=".pyo"&&Path.GetExtension(file)!=".part")yield return file;
  foreach(string child in Directory.GetDirectories(folder)) {
   string name=Path.GetFileName(child);if(name=="__pycache__"||name.StartsWith("~",StringComparison.Ordinal))continue;
   foreach(string file in Files(child))yield return file;
  }
 }
 public static void Stage(string target,string[] sources,CancellationToken cancel,Action<string,long,long> progress) {
  Directory.CreateDirectory(target);
  string claim=Path.Combine(target,"migration-runtime-source.json");string runtimeSource=null;
  if(File.Exists(claim)) runtimeSource=Json.Deserialize<string>(File.ReadAllText(claim));
  else if(!File.Exists(Path.Combine(target,"runtime","python.exe"))) {
   runtimeSource=sources.FirstOrDefault(s=>File.Exists(Path.Combine(s,"runtime","python.exe"))&&File.Exists(Path.Combine(s,"runtime-ok.json")))??sources.FirstOrDefault(s=>File.Exists(Path.Combine(s,"runtime","python.exe")));
   if(runtimeSource!=null)AtomicText(claim,Json.Serialize(runtimeSource));
  }
  var copy=new List<KeyValuePair<string,string>>();
  foreach(string source in sources) {
   if(Same(source,target)||Nested(target,source)||Nested(source,target))throw new IOException(AppText.T("Папки переноса не должны быть вложены друг в друга.","Source and destination folders cannot be nested."));
   foreach(string name in new[]{"models","audio","downloads","runtime","llama","cosy"}) {
    if(name=="runtime"&&(runtimeSource==null||!Same(source,runtimeSource)))continue;
    string folder=Path.Combine(source,name);
    foreach(string file in Files(folder)) {
     string destination=Path.GetFullPath(Path.Combine(target,name,file.Substring(folder.Length+1)));
     if(!Nested(destination,target))throw new IOException(AppText.T("Неверный путь при переносе.","Invalid migration path."));
     if(!File.Exists(destination))copy.Add(new KeyValuePair<string,string>(file,destination));
    }
   }
  }
  copy=copy.GroupBy(pair=>pair.Value,StringComparer.OrdinalIgnoreCase).Select(group=>group.First()).ToList();
  long total=copy.Sum(pair=>new FileInfo(pair.Key).Length),done=0;var last=DateTime.MinValue;var progressLock=new object();
  Parallel.ForEach(copy,new ParallelOptions {MaxDegreeOfParallelism=6,CancellationToken=cancel},pair=>{
   cancel.ThrowIfCancellationRequested();if(File.Exists(pair.Value)){Interlocked.Add(ref done,new FileInfo(pair.Key).Length);return;}
   Directory.CreateDirectory(Path.GetDirectoryName(pair.Value));string temporary=pair.Value+".storage-copy";
   using(var input=File.OpenRead(pair.Key))using(var output=new FileStream(temporary,FileMode.Create,FileAccess.Write,FileShare.None)) {
    byte[] buffer=new byte[256*1024];int count;
    while((count=input.Read(buffer,0,buffer.Length))>0) {
     cancel.ThrowIfCancellationRequested();output.Write(buffer,0,count);Interlocked.Add(ref done,count);
     lock(progressLock)if((DateTime.UtcNow-last).TotalMilliseconds>200){progress(AppText.T("Перенос в общее хранилище: ","Moving into shared storage: ")+Path.GetFileName(pair.Key),Interlocked.Read(ref done),total);last=DateTime.UtcNow;}
    }
   }
   File.Move(temporary,pair.Value);
  });
  if(runtimeSource!=null&&File.Exists(Path.Combine(runtimeSource,"runtime-ok.json"))&&!File.Exists(Path.Combine(target,"runtime-ok.json")))
   File.Copy(Path.Combine(runtimeSource,"runtime-ok.json"),Path.Combine(target,"runtime-ok.json"));
  progress(AppText.T("Файлы перенесены. Сохраняю библиотеку и историю…","Files migrated. Saving the library and history…"),total,total);
 }
 public static void Complete(string target,string[] sources) {
  string journal=Path.Combine(target,"migration-complete.json");var all=new List<string>();
  if(File.Exists(journal))all.AddRange(Json.Deserialize<string[]>(File.ReadAllText(journal)));
  all.AddRange(sources.Select(Normal));AtomicText(journal,Json.Serialize(all.Distinct(StringComparer.OrdinalIgnoreCase).ToArray()));
  string claim=Path.Combine(target,"migration-runtime-source.json");if(File.Exists(claim))File.Delete(claim);
 }
 static void AtomicText(string path,string text) {
  string temp=path+".tmp";File.WriteAllText(temp,text);if(File.Exists(path))File.Replace(temp,path,null);else File.Move(temp,path);
 }
}
