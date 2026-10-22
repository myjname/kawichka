using System;
using System.Collections.Generic;
using System.Drawing;
using System.IO;
using System.IO.Compression;
using System.Linq;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Web.Script.Serialization;

public static class PersonalPresets {
 public static string DefaultDescription(string app,string language){var item=Parse(File.ReadAllText(Path.Combine(app,"assets","character.json")));string key=language=="en"&&item.ContainsKey("description_en")?"description_en":"description";return Text(item,key);}
 public static bool IsDefaultDescription(string app,string text){string clean=(text??"").Replace("\r\n","\n").Trim();return new[]{"ru","en"}.Any(language=>DefaultDescription(app,language).Replace("\r\n","\n").Trim()==clean);}
 static readonly JavaScriptSerializer Json=new JavaScriptSerializer{MaxJsonLength=200000};
 static readonly string[] Kinds={"voice","character","visual"};
 static readonly string[] Images={".png",".webp",".gif",".jpg",".jpeg"};
 const long Limit=200*1024*1024;
 static string Normal(string path){return Path.GetFullPath(path).TrimEnd(Path.DirectorySeparatorChar,Path.AltDirectorySeparatorChar);}
 static bool Nested(string path,string root){return Normal(path).StartsWith(Normal(root)+Path.DirectorySeparatorChar,StringComparison.OrdinalIgnoreCase);}
 public static string Inside(string root,string name){
  if(string.IsNullOrWhiteSpace(name)||Path.IsPathRooted(name)||name.IndexOf(':')>=0||name.Replace('\\','/').Split('/').Any(p=>p=="."||p==".."||p.Length==0))throw new ArgumentException(L.T("Некорректный путь в пресете","Invalid preset path"));
  string target=Path.GetFullPath(Path.Combine(root,name.Replace('/',Path.DirectorySeparatorChar)));if(!Nested(target,root))throw new ArgumentException(AppText.T("Неверный путь пресета","Invalid preset path"));
  string current=target;while(Nested(current,root)){if((File.Exists(current)||Directory.Exists(current))&&(File.GetAttributes(current)&FileAttributes.ReparsePoint)!=0)throw new IOException(AppText.T("Ссылки в пресетах не поддерживаются","Preset links are not supported"));current=Path.GetDirectoryName(current);}
  if(Directory.Exists(root)&&(File.GetAttributes(root)&FileAttributes.ReparsePoint)!=0)throw new IOException(AppText.T("Ссылки в пресетах не поддерживаются","Preset links are not supported"));return target;
 }
 static string Folder(string root,string id){if(string.IsNullOrEmpty(id)||id.Length>100||id.Any(c=>!char.IsLetterOrDigit(c)))throw new ArgumentException(AppText.T("Неверный идентификатор пресета","Invalid preset ID"));return Inside(Path.Combine(root,"presets"),id);}
 static Dictionary<string,object> Parse(string text){var item=Json.DeserializeObject(text) as Dictionary<string,object>;if(item==null)throw new ArgumentException(AppText.T("Неверные данные пресета","Invalid preset metadata"));return item;}
 static string Text(Dictionary<string,object> item,string key){object value;return item.TryGetValue(key,out value)&&value is string?(string)value:"";}
 static Dictionary<string,object>[] Refs(Dictionary<string,object> item){object value;if(!item.TryGetValue("references",out value)||!(value is object[]))throw new ArgumentException(AppText.T("Неверный список голосовых записей","Invalid voice references"));return ((object[])value).Select(v=>v as Dictionary<string,object>??throwPreset()).ToArray();}
 static Dictionary<string,object> throwPreset(){throw new ArgumentException(AppText.T("Неверная голосовая запись","Invalid voice reference"));}
 static Dictionary<string,object> Clean(Dictionary<string,object> item,string kind){
  if(!Kinds.Contains(kind)||Text(item,"kind")!=kind||!item.ContainsKey("version")||!(item["version"] is int)||(int)item["version"]!=1)throw new ArgumentException(L.T("Несовместимый пресет","Incompatible preset"));
  string name=Text(item,"name").Trim();if(name.Length<1||name.Length>80)throw new ArgumentException(L.T("Название должно содержать 1–80 символов","Use 1–80 characters for the name"));
  var result=new Dictionary<string,object>{{"version",1},{"kind",kind},{"name",name}};
  if(kind=="character"){string description=Text(item,"description").Trim();if(description.Length<1||description.Length>4000)throw new ArgumentException(L.T("Описание должно содержать 1–4000 символов","Use 1–4000 characters for the description"));result["description"]=description;}
  else if(kind=="visual"){string file=Text(item,"file"),layout=Text(item,"layout");if(layout.Length==0)layout="auto";if(!Images.Contains(Path.GetExtension(file).ToLowerInvariant())||!new[]{"auto","image","codex"}.Contains(layout))throw new ArgumentException(AppText.T("Используйте PNG, WebP, GIF или JPEG и поддерживаемый вид отображения","Use PNG, WebP, GIF or JPEG with a valid layout"));result["file"]=file;result["layout"]=layout;}
  else{var refs=Refs(item);if(refs.Length<1||refs.Length>8)throw new ArgumentException(L.T("Нужно от 1 до 8 голосовых записей","Use 1–8 voice references"));result["references"]=refs.Select(r=>{string file=Text(r,"file"),transcript=Text(r,"transcript");if(!new[]{".wav",".flac"}.Contains(Path.GetExtension(file).ToLowerInvariant())||transcript.Length>4000)throw new ArgumentException(AppText.T("Неверная голосовая запись","Invalid voice reference"));return new Dictionary<string,object>{{"file",file},{"transcript",transcript}};}).ToArray();}
  return result;
 }
 static string[] Files(Dictionary<string,object> item){return Text(item,"kind")=="voice"?Refs(item).Select(r=>Text(r,"file")).ToArray():Text(item,"kind")=="visual"?new[]{Text(item,"file")}:new string[0];}
 static void ValidateFiles(string folder,Dictionary<string,object> item){foreach(string file in Files(item)){string path=Inside(folder,file);if(!File.Exists(path)||new FileInfo(path).Length>Limit)throw new ArgumentException(L.T("Файл пресета отсутствует или слишком велик","A preset file is missing or too large"));}}
 static void Write(string folder,Dictionary<string,object> item){string file=Inside(folder,"preset.json"),temp=file+".part";File.WriteAllText(temp,Json.Serialize(item),new UTF8Encoding(false));if(File.Exists(file))File.Replace(temp,file,null);else File.Move(temp,file);}
 public static Dictionary<string,object> Read(string root,string id,string kind){string folder=Folder(root,id),file=Inside(folder,"preset.json");if(new FileInfo(file).Length>150000)throw new ArgumentException(AppText.T("Файл описания пресета слишком велик","Preset metadata is too large"));var result=Clean(Parse(File.ReadAllText(file,Encoding.UTF8)),kind);ValidateFiles(folder,result);return result;}
 public static List<Dictionary<string,object>> List(string root){var result=new List<Dictionary<string,object>>();string parent=Path.Combine(root,"presets");if(!Directory.Exists(parent))return result;foreach(string folder in Directory.GetDirectories(parent)){try{string id=Path.GetFileName(folder),file=Inside(Folder(root,id),"preset.json");if(!File.Exists(file)||new FileInfo(file).Length>150000)continue;var raw=Parse(File.ReadAllText(file));var item=Read(root,id,Text(raw,"kind"));result.Add(new Dictionary<string,object>{{"id",id},{"kind",item["kind"]},{"name",item["name"]}});}catch(ArgumentException){}catch(IOException){}catch(UnauthorizedAccessException){}catch(InvalidOperationException){}}return result.OrderBy(p=>Text(p,"name")).ToList();}
 public static string FilePath(string root,string id,string kind){if(string.IsNullOrEmpty(id))return "";var item=Read(root,id,kind);return Inside(Folder(root,id),kind=="character"?"preset.json":Text(item,"file"));}
 public static string Description(string root,string id){return string.IsNullOrEmpty(id)?"":Text(Read(root,id,"character"),"description");}
 static void ValidateVisual(string file){using(var image=Image.FromFile(file)){if((long)image.Width*image.Height>40000000)throw new ArgumentException(L.T("Изображение слишком большое","Image is too large"));}}
 static async Task<string> CopyVisual(string root,string source,string folder,CancellationToken cancel,Action<string> progress){string extension=Path.GetExtension(source).ToLowerInvariant();if(!Images.Contains(extension))throw new ArgumentException(AppText.T("Используйте PNG, WebP, GIF или JPEG","Use PNG, WebP, GIF or JPEG"));string file="visual"+(extension==".webp"?".png":extension),target=Inside(folder,file);
  if(extension==".webp"){string decoder=await MediaTools.Ensure(root,cancel,progress);await Task.Run(delegate{MediaTools.Run(decoder,"-i "+MediaTools.Q(source)+" -frames:v 1 "+MediaTools.Q(target),cancel);},cancel);}else File.Copy(source,target);
  ValidateVisual(target);return file;
 }
 public static async Task<string> Import(string root,string kind,string[] files,string transcript,CancellationToken cancel,Action<string> progress){
  if(!Kinds.Contains(kind)||files==null||files.Length<1||files.Length>(kind=="voice"?8:1)||files.Any(f=>!File.Exists(f)||new FileInfo(f).Length>Limit))throw new ArgumentException(L.T("Выбери допустимые файлы: до 200 МБ каждый, до 8 записей голоса","Choose valid files: up to 200 MB each, up to 8 voice references"));
  cancel.ThrowIfCancellationRequested();string id=Guid.NewGuid().ToString("N"),folder=Folder(root,id);Directory.CreateDirectory(folder);bool success=false;
  try{
   if(files.Length==1&&Path.GetExtension(files[0]).Equals(".kavychka",StringComparison.OrdinalIgnoreCase)){await ImportPackage(root,files[0],folder,kind,cancel,progress);}
   else{
    string name=Path.GetFileNameWithoutExtension(files[0]);var item=new Dictionary<string,object>{{"version",1},{"kind",kind},{"name",name.Substring(0,Math.Min(80,name.Length))}};
    if(kind=="character"){if(new FileInfo(files[0]).Length>20000)throw new ArgumentException(AppText.T("Описание слишком велико","Description is too large"));item["description"]=File.ReadAllText(files[0],Encoding.UTF8).Trim();}
    else if(kind=="visual")item["file"]=await CopyVisual(root,files[0],folder,cancel,progress);
    else{string decoder=await MediaTools.Ensure(root,cancel,progress);var refs=new List<object>();for(int i=0;i<files.Length;i++){cancel.ThrowIfCancellationRequested();progress(L.T("Подготавливаю запись ","Preparing reference ")+(i+1)+" / "+files.Length);string relative="ref"+i,part=Inside(folder,relative);Directory.CreateDirectory(part);int index=i;await Task.Run(delegate{MediaTools.NormalizeVoice(decoder,files[index],part,cancel);},cancel);refs.Add(new Dictionary<string,object>{{"file",relative+"/reference.flac"},{"transcript",files.Length==1?transcript:""}});}item["references"]=refs.ToArray();}
    item=Clean(item,kind);ValidateFiles(folder,item);Write(folder,item);
   }
   cancel.ThrowIfCancellationRequested();success=true;return id;
  }finally{if(!success)DeleteFolder(root,id);}
 }
 static async Task ImportPackage(string root,string source,string folder,string kind,CancellationToken cancel,Action<string> progress){
  using(var archive=ZipFile.OpenRead(source)){
   if(archive.Entries.Count>150||archive.Entries.Sum(e=>e.Length)>Limit)throw new ArgumentException(AppText.T("Архив пресета слишком велик","Preset archive is too large"));var entries=new Dictionary<string,ZipArchiveEntry>(StringComparer.OrdinalIgnoreCase);
   foreach(var entry in archive.Entries){cancel.ThrowIfCancellationRequested();string name=entry.FullName.TrimEnd('/');Inside(folder,name);if(entry.Name.Length==0)continue;if(!new[]{".json",".wav",".flac",".png",".gif",".webp",".jpg",".jpeg"}.Contains(Path.GetExtension(name).ToLowerInvariant())||entries.ContainsKey(name))throw new ArgumentException(AppText.T("Неверная запись архива пресета","Invalid preset archive entry"));entries[name]=entry;}
   ZipArchiveEntry manifest;if(!entries.TryGetValue("preset.json",out manifest)||manifest.Length>150000)throw new ArgumentException(AppText.T("Не найдено описание пресета","Missing preset metadata"));Dictionary<string,object> item;using(var reader=new StreamReader(manifest.Open(),Encoding.UTF8))item=Clean(Parse(reader.ReadToEnd()),kind);
   var references=Files(item);foreach(string name in references)Inside(folder,name);
   // Only referenced media is extracted. Extra preferences/credentials in an
   // archive or its metadata cannot become part of this component preset.
   for(int i=0;i<references.Length;i++){string name=references[i];ZipArchiveEntry entry;if(!entries.TryGetValue(name,out entry))throw new ArgumentException(AppText.T("Файл пресета отсутствует","Preset media is missing"));string temp=Inside(folder,"input"+i+Path.GetExtension(name));entry.ExtractToFile(temp);
    if(kind=="voice"){string decoder=await MediaTools.Ensure(root,cancel,progress),part=Inside(folder,"ref"+i);Directory.CreateDirectory(part);await Task.Run(delegate{MediaTools.NormalizeVoice(decoder,temp,part,cancel);},cancel);Refs(item)[i]["file"]="ref"+i+"/reference.flac";}
    else item["file"]=await CopyVisual(root,temp,folder,cancel,progress);File.Delete(temp);
   }
   Write(folder,item);
  }
 }
 public static string SaveCharacter(string root,string id,string name,string description){
  var item=Clean(new Dictionary<string,object>{{"version",1},{"kind","character"},{"name",name},{"description",description}},"character");
  if(string.IsNullOrEmpty(id)){id=Guid.NewGuid().ToString("N");Directory.CreateDirectory(Folder(root,id));}else Read(root,id,"character");
  Write(Folder(root,id),item);return id;
 }
 public static string CloneDefaultVoice(string root,string app,string name,string transcript){var item=Clean(new Dictionary<string,object>{{"version",1},{"kind","voice"},{"name",name},{"references",new object[]{new Dictionary<string,object>{{"file","reference.wav"},{"transcript",transcript}}}}},"voice");string source=Path.Combine(app,"assets","voice","default-en.wav");double seconds=MediaTools.WaveSeconds(source);if(seconds<1||seconds>30)throw new ArgumentException(AppText.T("Неверная стандартная голосовая запись","Invalid default voice reference"));string id=Guid.NewGuid().ToString("N"),folder=Folder(root,id);Directory.CreateDirectory(folder);File.Copy(source,Inside(folder,"reference.wav"));Write(folder,item);return id;}
 public static void Update(string root,string id,string kind,string name,string description,string[] transcripts,string layout=null){var item=Read(root,id,kind);item["name"]=name;if(kind=="character")item["description"]=description;if(kind=="voice"){var refs=Refs(item);if(transcripts==null||transcripts.Length!=refs.Length)throw new ArgumentException(AppText.T("Количество записей изменилось","Reference count changed"));for(int i=0;i<refs.Length;i++)refs[i]["transcript"]=transcripts[i];}if(kind=="visual"&&layout!=null){using(var image=Image.FromFile(FilePath(root,id,kind)))if(layout=="codex"&&!SpriteAtlas.Matches(image))throw new ArgumentException(AppText.T("Лист спрайтов Codex должен иметь размер 1536 × 1872","Codex sprite sheet must be 1536 x 1872"));item["layout"]=layout;}Write(Folder(root,id),Clean(item,kind));}
 public static string[] Transcripts(string root,string id){return Refs(Read(root,id,"voice")).Select(r=>Text(r,"transcript")).ToArray();}
 public static async Task<string> PreviewAudio(string root,string id,int index,CancellationToken cancel,Action<string> progress){
  var refs=Refs(Read(root,id,"voice"));if(index<0||index>=refs.Length)throw new ArgumentException(AppText.T("Неверный номер голосовой записи","Invalid voice reference index"));string folder=Folder(root,id),source=Inside(folder,Text(refs[index],"file")),preview=Inside(folder,"preview"+index+".wav");
  if(Path.GetExtension(source).Equals(".wav",StringComparison.OrdinalIgnoreCase)){MediaTools.WaveSeconds(source);return source;}
  if(!File.Exists(preview)){string decoder=await MediaTools.Ensure(root,cancel,progress);await Task.Run(delegate{MediaTools.Run(decoder,"-i "+MediaTools.Q(source)+" -vn -ac 1 -ar 24000 -c:a pcm_s16le "+MediaTools.Q(preview),cancel);MediaTools.WaveSeconds(preview);},cancel);}return preview;
 }
 public static void Export(string root,string id,string kind,string target){
  string folder=Folder(root,id);if(Nested(target,Path.Combine(root,"presets")))throw new ArgumentException(AppText.T("Выберите место экспорта вне папки пресетов","Choose an export location outside your presets folder"));var item=Read(root,id,kind);string temp=Path.GetFullPath(target)+".part";
  try{using(var stream=new FileStream(temp,FileMode.Create,FileAccess.ReadWrite,FileShare.None))using(var archive=new ZipArchive(stream,ZipArchiveMode.Create)){using(var writer=new StreamWriter(archive.CreateEntry("preset.json").Open(),new UTF8Encoding(false)))writer.Write(Json.Serialize(item));foreach(string file in Files(item).Distinct())archive.CreateEntryFromFile(Inside(folder,file),file.Replace('\\','/'));}if(File.Exists(target))File.Replace(temp,target,null);else File.Move(temp,target);}finally{if(File.Exists(temp))File.Delete(temp);}
 }
 public static void Delete(string root,string id,string kind){Read(root,id,kind);DeleteFolder(root,id);}
 static void DeleteFolder(string root,string id){string folder=Folder(root,id),parent=Path.Combine(root,"presets");if(!Directory.Exists(folder))return;
  // Validate the resolved target and every child before recursive deletion.
  if(!Nested(folder,parent)||!string.Equals(Path.GetFileName(folder),id,StringComparison.Ordinal))throw new IOException(AppText.T("Неверный путь удаления пресета","Invalid preset cleanup path"));ValidateTree(folder,folder);Directory.Delete(folder,true);
 }
 static void ValidateTree(string root,string folder){foreach(string item in Directory.GetFileSystemEntries(folder)){Inside(root,item.Substring(Normal(root).Length+1));if(Directory.Exists(item))ValidateTree(root,item);}}
}
