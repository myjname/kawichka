using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Security.Cryptography;
using System.Text;
using System.Web.Script.Serialization;

public static class LegacyPersonalization {
 static readonly JavaScriptSerializer Json=new JavaScriptSerializer();
 static Dictionary<string,object> Read(string file){return (Dictionary<string,object>)Json.DeserializeObject(File.ReadAllText(file,Encoding.UTF8));}
 static string Hash(byte[] data){using(var hash=SHA256.Create())return BitConverter.ToString(hash.ComputeHash(data)).Replace("-","").ToLowerInvariant();}
 static string Within(string folder,string name){string path=Path.GetFullPath(Path.Combine(folder,name));if(!path.StartsWith(Path.GetFullPath(folder)+Path.DirectorySeparatorChar,StringComparison.OrdinalIgnoreCase))throw new IOException(AppText.T("Неверный путь прежней голосовой записи","Invalid legacy reference path"));return path;}
 public static string SaveCharacter(string data,string description,string name){
  if(string.IsNullOrWhiteSpace(description)||description.Length>4000)throw new ArgumentException(AppText.T("Описание должно содержать 1–4000 символов","A character description needs 1–4000 characters"));
  string id="character"+Hash(Encoding.UTF8.GetBytes(description)).Substring(0,24),folder=Path.Combine(data,"presets",id);Directory.CreateDirectory(folder);string file=Path.Combine(folder,"preset.json");
  if(!File.Exists(file))File.WriteAllText(file,Json.Serialize(new{version=1,kind="character",name=name,description=description}),Encoding.UTF8);return id;
 }
 static string SaveVoice(string voiceFolder,string data,List<Dictionary<string,object>> refs,string name){
  if(refs.Count>8)throw new IOException(AppText.T("В прежнем голосовом пресете больше восьми записей","A legacy voice preset contains more than eight references"));
  var paths=refs.Select(r=>Within(voiceFolder,Convert.ToString(r["file"]))).ToArray();if(paths.Any(p=>!File.Exists(p)))throw new IOException(AppText.T("Прежняя голосовая запись отсутствует; установка её не заменяла","A legacy voice reference is missing; installation has not replaced it"));
  string id="legacyvoice"+Hash(Encoding.UTF8.GetBytes(string.Join("",paths.Select(p=>Hash(File.ReadAllBytes(p)))))).Substring(0,16),folder=Path.Combine(data,"presets",id);Directory.CreateDirectory(folder);var exported=new List<object>();
  for(int i=0;i<paths.Length;i++){string file="reference"+i+Path.GetExtension(paths[i]);string target=Path.Combine(folder,file);if(!File.Exists(target))File.Copy(paths[i],target);exported.Add(new{file=file,transcript=refs[i].ContainsKey("transcript")?Convert.ToString(refs[i]["transcript"]):""});}
  string manifest=Path.Combine(folder,"preset.json");if(!File.Exists(manifest))File.WriteAllText(manifest,Json.Serialize(new{version=1,kind="voice",name=name,references=exported.ToArray()}),Encoding.UTF8);return id;
 }
 public static void ImportCaptured(string captured,string data){
  string marker=Path.Combine(captured,"legacy-personalization.json");if(!File.Exists(marker))return;var ids=Json.Deserialize<Dictionary<string,string>>(File.ReadAllText(marker));
  foreach(var pair in ids){if(!new[]{"voice","original_voice","character"}.Contains(pair.Key)||pair.Value.Any(c=>!char.IsLetterOrDigit(c)))throw new IOException(AppText.T("Неверные сохранённые настройки персонажа","Invalid captured personalization"));string source=Within(Path.Combine(captured,"presets"),pair.Value),target=Within(Path.Combine(data,"presets"),pair.Value);if(Directory.Exists(target))continue;Directory.CreateDirectory(target);
   foreach(string file in Directory.GetFiles(source)){if(!new[]{".json",".wav",".flac"}.Contains(Path.GetExtension(file).ToLowerInvariant())||(File.GetAttributes(file)&FileAttributes.ReparsePoint)!=0)throw new IOException(AppText.T("Неверный сохранённый файл пресета","Invalid captured preset file"));File.Copy(file,Within(target,Path.GetFileName(file)));}
  }
  Directory.CreateDirectory(data);string destination=Path.Combine(data,"legacy-personalization.json");if(!File.Exists(destination))File.Copy(marker,destination);
 }
 public static Dictionary<string,string> Capture(string app,string data){
  var result=new Dictionary<string,string>();string marker=Path.Combine(data,"legacy-personalization.json");
  if(File.Exists(marker))result=Json.Deserialize<Dictionary<string,string>>(File.ReadAllText(marker));
  string character=Path.Combine(app,"assets","character.json"),voiceFolder=Path.Combine(app,"assets","voice");
  if(File.Exists(character)){
   var item=Read(character);if(!item.ContainsKey("public_default")||!Convert.ToBoolean(item["public_default"])){
    string description=Convert.ToString(item["description"]),id="legacycharacter"+Hash(Encoding.UTF8.GetBytes(description)).Substring(0,16),folder=Path.Combine(data,"presets",id);Directory.CreateDirectory(folder);
    string name=item.ContainsKey("name")?Convert.ToString(item["name"]):"Imported character";
    string manifest=Path.Combine(folder,"preset.json");if(!File.Exists(manifest))File.WriteAllText(manifest,Json.Serialize(new{version=1,kind="character",name=name,description=description}),Encoding.UTF8);result["character"]=id;
   }
  }
  string profileFile=Path.Combine(voiceFolder,"profile.json"),oldRefs=Path.Combine(voiceFolder,"cosy-references.json");
  if(File.Exists(profileFile)){
   var profile=Read(profileFile);bool personal=!profile.ContainsKey("public_default")||!Convert.ToBoolean(profile["public_default"]);
   var refs=new List<Dictionary<string,object>>();
   if(File.Exists(oldRefs)){
    var metadata=Read(oldRefs);if(metadata.ContainsKey("references")&&(!metadata.ContainsKey("public_default")||!Convert.ToBoolean(metadata["public_default"])))foreach(object value in (object[])metadata["references"]){var entry=(Dictionary<string,object>)value;if(entry.ContainsKey("file"))refs.Add(entry);}
   }
   if(refs.Count==0&&personal&&profile.ContainsKey("file"))refs.Add(new Dictionary<string,object>{{"file",profile["file"]},{"transcript",profile.ContainsKey("transcript")?profile["transcript"]:""}});
   if(refs.Count>0)result["voice"]=SaveVoice(voiceFolder,data,refs,AppText.T("Прежний голос","Imported voice"));
   if(personal&&profile.ContainsKey("file")&&refs.All(r=>!string.Equals(Convert.ToString(r["file"]),Convert.ToString(profile["file"]),StringComparison.OrdinalIgnoreCase))&&File.Exists(Within(voiceFolder,Convert.ToString(profile["file"]))))
    result["original_voice"]=SaveVoice(voiceFolder,data,new List<Dictionary<string,object>>{new Dictionary<string,object>{{"file",profile["file"]},{"transcript",profile.ContainsKey("transcript")?profile["transcript"]:""}}},AppText.T("Прежний голос · исходная запись","Imported voice · original reference"));
  }
  if(result.Count>0){Directory.CreateDirectory(data);File.WriteAllText(marker,Json.Serialize(result),Encoding.UTF8);}return result;
 }
}
