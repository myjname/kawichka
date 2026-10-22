using System;
using System.Collections.Generic;
using System.IO;
using System.IO.Compression;
using System.Linq;
using System.Security.Cryptography;
using System.Text;
using System.Web.Script.Serialization;

public static class InstallTransactionTests {
 static readonly JavaScriptSerializer Json=new JavaScriptSerializer();
 static int count;
 static string root;
 static void Check(bool ok,string text){if(!ok)throw new Exception(text);Console.WriteLine("PASS "+text);count++;}
 static void Write(string file,string text){Directory.CreateDirectory(Path.GetDirectoryName(file));File.WriteAllText(file,text,new UTF8Encoding(false));}
 static string Hash(byte[] data){using(var hash=SHA256.Create())return BitConverter.ToString(hash.ComputeHash(data)).Replace("-","").ToLowerInvariant();}
 static Dictionary<string,string> Files(string version){return new Dictionary<string,string>{{"Kavychka.exe",version},{"backend/server.py","server "+version},{"backend/health_check.py","health "+version},{"assets/character.json","{\"public_default\":true,\"description\":\"Generic\"}"},{"assets/voice/profile.json","{\"public_default\":true}"}};}
 static MemoryStream Package(Dictionary<string,string> files,string extra=null,bool badHash=false,string missing=null){
  var output=new MemoryStream();using(var zip=new ZipArchive(output,ZipArchiveMode.Create,true)){
   var manifest=new PackageManifest{format=1,version="2.0.0.0",files=files.Select(f=>new PackageFile{path=f.Key,bytes=Encoding.UTF8.GetByteCount(f.Value),sha256=badHash?new string('0',64):Hash(Encoding.UTF8.GetBytes(f.Value))}).ToArray()};
   foreach(var file in files){if(file.Key==missing)continue;using(var writer=new StreamWriter(zip.CreateEntry(file.Key).Open(),new UTF8Encoding(false)))writer.Write(file.Value);}
   using(var writer=new StreamWriter(zip.CreateEntry("package.json").Open(),new UTF8Encoding(false)))writer.Write(Json.Serialize(manifest));
   if(extra!=null)using(var writer=new StreamWriter(zip.CreateEntry(extra).Open()))writer.Write("unexpected");
  }output.Position=0;return output;
 }
 static void Apply(string name,Dictionary<string,string> files,Action<string,string,string> health){using(var package=Package(files))InstallTransaction.Apply(package,Path.Combine(root,name,"app"),Path.Combine(root,name,"data"),delegate{},health);}
 static bool Reject(Action action){try{action();return false;}catch(InvalidDataException){return true;}}
 public static int Main(){root=Path.GetFullPath(Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"..","..","work","install-tests-"+Guid.NewGuid().ToString("N")));Directory.CreateDirectory(root);bool success=false;
  try{
   string app=Path.Combine(root,"normal/app"),data=Path.Combine(root,"normal/data");Write(Path.Combine(app,"notes.txt"),"mine");Write(Path.Combine(data,"models/model.bin"),"weights");Write(Path.Combine(data,"audio/keep.wav"),"recording");Write(Path.Combine(data,"settings.json"),"old preferences");
   var old=Files("old");old["backend/obsolete.py"]="old module";Apply("normal",old,delegate{});
   Apply("normal",Files("new"),delegate(string a,string d,string v){Write(Path.Combine(d,"settings.json"),"migrated preferences");});
   Check(File.ReadAllText(Path.Combine(app,"Kavychka.exe"))=="new"&&!File.Exists(Path.Combine(app,"backend/obsolete.py")),"upgrade replaces binaries and removes obsolete owned files");
   Check(File.ReadAllText(Path.Combine(app,"notes.txt"))=="mine"&&File.ReadAllText(Path.Combine(data,"models/model.bin"))=="weights"&&File.ReadAllText(Path.Combine(data,"audio/keep.wav"))=="recording","update preserves unrelated files, models and audio");
   Check(File.ReadAllText(Path.Combine(data,"settings.json"))=="migrated preferences"&&!File.Exists(Path.Combine(app,".kavychka-install.json")),"successful migration commits preferences");
   Check(Directory.GetDirectories(Path.GetDirectoryName(app),".kavychka-setup-*").Length==2,"previous program and preference snapshots are retained");
   Write(Path.Combine(data,"library.sqlite3"),"old database");Write(Path.Combine(data,"library.sqlite3-wal"),"old WAL");Write(Path.Combine(data,"reminder-state.json"),"pending reminder and timer");
   bool failed=false;try{Apply("normal",Files("broken"),delegate(string a,string d,string v){Write(Path.Combine(d,"settings.json"),"broken preferences");Write(Path.Combine(d,"library.sqlite3"),"broken database");Write(Path.Combine(d,"reminder-state.json"),"incompatible migrated reminder");File.Delete(Path.Combine(d,"library.sqlite3-wal"));throw new IOException("simulated startup failure");});}catch(IOException){failed=true;}
   Check(failed&&File.ReadAllText(Path.Combine(app,"Kavychka.exe"))=="new"&&File.ReadAllText(Path.Combine(data,"settings.json"))=="migrated preferences","failed startup restores previous binary and preferences");
   Check(File.ReadAllText(Path.Combine(data,"library.sqlite3"))=="old database"&&File.ReadAllText(Path.Combine(data,"library.sqlite3-wal"))=="old WAL","failed migration restores SQLite with its WAL");
   Check(File.ReadAllText(Path.Combine(data,"reminder-state.json"))=="pending reminder and timer","failed migration restores the previous reminder journal");
   Check(!File.Exists(Path.Combine(app,".kavychka-install.json")),"completed rollback clears the recovery journal");
   string lockedApp=Path.Combine(root,"locked/app"),lockedData=Path.Combine(root,"locked/data");Write(Path.Combine(lockedData,"settings.json"),"unchanged settings");Apply("locked",Files("stable"),delegate{});
   using(var locked=new FileStream(Path.Combine(lockedData,"settings.json"),FileMode.Open,FileAccess.Read,FileShare.Read)){
    failed=false;try{Apply("locked",Files("broken"),delegate{throw new IOException("settings cannot be saved");});}catch(IOException){failed=true;}
    Check(failed&&File.ReadAllText(Path.Combine(lockedApp,"Kavychka.exe"))=="stable"&&!File.Exists(Path.Combine(lockedApp,".kavychka-install.json")),"rollback completes when an unchanged preferences file is locked against writes");
   }
   Apply("normal",Files("third"),delegate{});Apply("normal",Files("fourth"),delegate{});Check(Directory.GetDirectories(Path.GetDirectoryName(app),".kavychka-setup-*").Length==2,"successful updates retain only two recent backups");
   foreach(string scenario in new[]{"traversal","corrupt","missing"}){
    string a=Path.Combine(root,scenario,"app"),d=Path.Combine(root,scenario,"data");Write(Path.Combine(a,"Kavychka.exe"),"working");Write(Path.Combine(d,"settings.json"),"untouched");
    using(var package=Package(Files("rejected"),scenario=="traversal"?"../escape.txt":null,scenario=="corrupt",scenario=="missing"?"backend/server.py":null))Check(Reject(delegate{InstallTransaction.Apply(package,a,d,delegate{},delegate{});}),"reject "+scenario+" package");
    Check(File.ReadAllText(Path.Combine(a,"Kavychka.exe"))=="working"&&File.ReadAllText(Path.Combine(d,"settings.json"))=="untouched","invalid "+scenario+" package leaves working install unchanged");
   }
   string recoverApp=Path.Combine(root,"interrupted/app"),recoverData=Path.Combine(root,"interrupted/data"),workspace=Path.Combine(root,"interrupted/.kavychka-setup-recovery");
   Write(Path.Combine(recoverApp,"Kavychka.exe"),"partial overwrite");Write(Path.Combine(recoverData,"settings.json"),"partial migration");Write(Path.Combine(workspace,"program/Kavychka.exe"),"stable app");Write(Path.Combine(workspace,"preferences/settings.json"),"stable settings");
   var journal=new InstallJournal{target=recoverApp,data=recoverData,workspace=workspace,state="prepared",program=new[]{new InstallSnapshot{path="Kavychka.exe",existed=true}},preferences=new[]{new InstallSnapshot{path="settings.json",existed=true}}};Write(Path.Combine(recoverApp,".kavychka-install.json"),Json.Serialize(journal));
   InstallTransaction.Recover(recoverApp,recoverData,delegate{});Check(File.ReadAllText(Path.Combine(recoverApp,"Kavychka.exe"))=="stable app"&&File.ReadAllText(Path.Combine(recoverData,"settings.json"))=="stable settings"&&!Directory.Exists(workspace),"interrupted update recovers from durable snapshots");
   journal.workspace=Path.Combine(root,"unrelated");Write(Path.Combine(journal.workspace,"keep.txt"),"keep");Write(Path.Combine(recoverApp,".kavychka-install.json"),Json.Serialize(journal));
   Check(Reject(delegate{InstallTransaction.Recover(recoverApp,recoverData,delegate{});})&&File.Exists(Path.Combine(journal.workspace,"keep.txt")),"recovery refuses an unrelated workspace");
   string legacy=Path.Combine(root,"legacy/app"),legacyData=Path.Combine(root,"legacy/data");Write(Path.Combine(legacy,"assets/character.json"),"{\"description\":\"My custom character\",\"name\":\"Personal\"}");Write(Path.Combine(legacy,"assets/voice/profile.json"),"{\"file\":\"original.wav\",\"transcript\":\"test\"}");Write(Path.Combine(legacy,"assets/voice/cosy-references.json"),"{\"references\":[{\"file\":\"a.wav\",\"source\":\"private URL\"},{\"file\":\"b.wav\"}]}");Write(Path.Combine(legacy,"assets/voice/a.wav"),"reference A");Write(Path.Combine(legacy,"assets/voice/b.wav"),"reference B");
   Write(Path.Combine(legacy,"assets/voice/original.wav"),"original Qwen reference");
   var ids=LegacyPersonalization.Capture(legacy,legacyData);string voice=File.ReadAllText(Path.Combine(legacyData,"presets",ids["voice"],"preset.json"));
   Check(ids.Count==3&&voice.Contains("reference0.wav")&&voice.Contains("reference1.wav")&&!voice.Contains("private URL"),"legacy references migrate independently without source URLs");
   Check(File.ReadAllText(Path.Combine(legacyData,"presets",ids["character"],"preset.json")).Contains("My custom character"),"legacy personality is retained outside program files");
   Check(File.ReadAllText(Path.Combine(legacyData,"presets",ids["original_voice"],"reference0.wav"))=="original Qwen reference"&&File.ReadAllText(Path.Combine(legacyData,"presets",ids["original_voice"],"preset.json")).Contains("test"),"the separate original voice and transcript survive alongside alternate Cosy references");
   string capturedData=Path.Combine(root,"captured/data");LegacyPersonalization.ImportCaptured(legacyData,capturedData);LegacyPersonalization.ImportCaptured(legacyData,capturedData);
   Check(Directory.GetDirectories(Path.Combine(capturedData,"presets")).Length==3&&File.ReadAllText(Path.Combine(capturedData,"presets",ids["original_voice"],"reference0.wav"))=="original Qwen reference","captured source builds import every independent preset once");
   string release=Json.Serialize(new{draft=false,prerelease=false,tag_name="v2.0.1",name="Release",assets=new[]{new{name="Kavychka-Setup.exe",browser_download_url="https://github.com/myjname/kawichka/releases/download/v2.0.1/Kavychka-Setup.exe",digest="sha256:"+new string('a',64),size=12345}}});
   Check(AppRelease.Parse(release).Version==new Version(2,0,1,0),"GitHub release arrays parse and versions normalize");
   Check(Reject(delegate{AppRelease.Parse(release.Replace("https://github.com/","https://example.com/"));}),"reject updates from an unexpected host");
   Check(Reject(delegate{AppRelease.Parse(release.Replace("sha256:","unknown:"));}),"reject updates without a SHA-256 digest");
   string verified=Path.Combine(root,"installer.exe");Write(verified,"verified installer");var candidate=new AppRelease{Bytes=new FileInfo(verified).Length,Sha256=Hash(File.ReadAllBytes(verified))};Check(AppUpdates.Verify(verified,candidate),"accept matching installer digest");Write(verified,"modified installer");Check(!AppUpdates.Verify(verified,candidate),"reject modified installer");
   Console.WriteLine("PASS "+count+" install/update checks");success=true;return 0;
  }catch(Exception error){Console.Error.WriteLine(error);Console.Error.WriteLine("Fixtures retained at "+root);return 1;}
  finally{string work=Path.GetFullPath(Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"..","..","work"));if(success&&root.StartsWith(work+Path.DirectorySeparatorChar,StringComparison.OrdinalIgnoreCase)&&Path.GetFileName(root).StartsWith("install-tests-",StringComparison.Ordinal))Directory.Delete(root,true);}
 }
}
