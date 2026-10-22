using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Security.Cryptography;
using System.Text;
using System.Web.Script.Serialization;

public sealed class BuiltinPhrase {
 public string Id,Slot,Category,Text,CreatedAt,UsedAt="",Kind="planned";
 public bool Followup,Approved=true,Rejected,Deleted,Active=true;
}
public sealed class BuiltinDay {
 public int Version=1;
 public string Day,Language;
 public List<BuiltinPhrase> Rows=new List<BuiltinPhrase>();
}

// The basic mode does not need Python or SQLite. Each day/language has its own
// atomic file; previous files remain as text history, including excluded rows.
public sealed class BuiltinDays {
 readonly string directory;
 readonly object gate=new object();
 readonly JavaScriptSerializer json=new JavaScriptSerializer{MaxJsonLength=32000000};
 static readonly string[] Categories={"break","breakfast","lunch","dinner","exercise"};
 public string[] HistoryWarnings=new string[0];
 public BuiltinDays(string root){directory=Path.Combine(Path.GetFullPath(root),"builtin-days");}
 void Guard(){if(Directory.Exists(directory)&&(File.GetAttributes(directory)&FileAttributes.ReparsePoint)!=0)throw new IOException(AppText.T("Ссылки на папки истории фраз не поддерживаются","Linked phrase history folders are not supported"));}
 string PathFor(Settings s,DateTime now){Guard();if(s.SpeechLanguage!="ru"&&s.SpeechLanguage!="en")throw new ArgumentException(AppText.T("Неподдерживаемый язык напоминаний","Unsupported reminder language"));return Path.Combine(directory,RitualSchedule.Local(s,now).ToString("yyyy-MM-dd",CultureInfo.InvariantCulture)+"."+s.SpeechLanguage+".json");}
 static bool DayName(string name){DateTime date;return name.Length==18&&(name.EndsWith(".ru.json",StringComparison.Ordinal)||name.EndsWith(".en.json",StringComparison.Ordinal))&&DateTime.TryParseExact(name.Substring(0,10),"yyyy-MM-dd",CultureInfo.InvariantCulture,DateTimeStyles.None,out date);}
 BuiltinDay ReadFile(string file){
  if((File.GetAttributes(file)&FileAttributes.ReparsePoint)!=0||new FileInfo(file).Length>32000000)throw new InvalidDataException(AppText.T("Неверная история готовых фраз","Invalid built-in phrase history"));
  var day=json.Deserialize<BuiltinDay>(File.ReadAllText(file));
  if(day==null||day.Version!=1||day.Rows==null||day.Rows.Count>50000||Path.GetFileName(file)!=day.Day+"."+day.Language+".json"||!DayName(Path.GetFileName(file)))throw new InvalidDataException(AppText.T("Неподдерживаемый файл дневного списка","Unsupported built-in day file"));
  var ids=new HashSet<string>(StringComparer.Ordinal);
  foreach(var row in day.Rows){Guid id;DateTime time;
   if(row==null||row.Id==null||!row.Id.StartsWith("stock-",StringComparison.Ordinal)||row.Id.Length!=38||!Guid.TryParseExact(row.Id.Substring(6),"N",out id)||!ids.Add(row.Id)||string.IsNullOrWhiteSpace(row.Slot)||row.Slot.Length>200||!Categories.Contains(row.Category)||string.IsNullOrWhiteSpace(row.Text)||row.Text.Length>400||!new[]{"planned","manual","fallback","external"}.Contains(row.Kind)||row.UsedAt==null||!DateTime.TryParse(row.CreatedAt,CultureInfo.InvariantCulture,DateTimeStyles.RoundtripKind,out time)||(row.UsedAt.Length>0&&!DateTime.TryParse(row.UsedAt,CultureInfo.InvariantCulture,DateTimeStyles.RoundtripKind,out time)))throw new InvalidDataException(AppText.T("Неверная готовая фраза","Invalid built-in phrase"));
  }
  return day;
 }
 BuiltinDay ReadDay(Settings s,DateTime now){string file=PathFor(s,now);return File.Exists(file)?ReadFile(file):new BuiltinDay{Day=RitualSchedule.Local(s,now).ToString("yyyy-MM-dd",CultureInfo.InvariantCulture),Language=s.SpeechLanguage};}
 void Write(Settings s,DateTime now,BuiltinDay day){string data=json.Serialize(day);if(day.Rows.Count>50000||Encoding.UTF8.GetByteCount(data)>31999997)throw new IOException(AppText.T("Дневной файл фраз слишком велик","Daily phrase file is too large"));Directory.CreateDirectory(directory);Guard();SettingsFile.Write(PathFor(s,now),data);}
 Dictionary<string,int> History(){
  Guard();var result=new Dictionary<string,int>(StringComparer.Ordinal);var warnings=new List<string>();
  if(Directory.Exists(directory))foreach(string file in Directory.GetFiles(directory,"*.json").Where(path=>DayName(Path.GetFileName(path)))){
   try{foreach(var row in ReadFile(file).Rows){int count;result.TryGetValue(row.Text,out count);result[row.Text]=count+1;}}
   catch(Exception error){if(!(error is IOException)&&!(error is UnauthorizedAccessException)&&!(error is ArgumentException)&&!(error is InvalidOperationException))throw;warnings.Add(Path.GetFileName(file)+": "+error.Message);}
  }
  HistoryWarnings=warnings.ToArray();return result;
 }
 static int Seed(string value){using(var hash=SHA256.Create())return (int)(BitConverter.ToUInt32(hash.ComputeHash(Encoding.UTF8.GetBytes(value)),0)&0x7fffffff);}
 BuiltinPhrase New(BuiltinDay day,string slot,string category,bool followup,bool approved,string kind,DateTime now,Dictionary<string,int> history,HashSet<string> today){
  string chosen=null;int best=int.MaxValue;
  foreach(string text in Fallbacks.Candidates(day.Language,category,followup,Seed(day.Day+"|"+day.Language+"|"+slot))){
   if(today.Contains(text))continue;int count;history.TryGetValue(text,out count);
   if(count<best){chosen=text;best=count;if(count==0)break;}
  }
  if(chosen==null)throw new InvalidOperationException(L.T("Готовые варианты на сегодня закончились. Для новых формулировок подключи текстовую модель.","Today's ready-made variants are exhausted. Choose a text model for new wording."));
  today.Add(chosen);int used;history.TryGetValue(chosen,out used);history[chosen]=used+1;
  return new BuiltinPhrase{Id="stock-"+Guid.NewGuid().ToString("N"),Slot=slot,Category=category,Followup=followup,Text=chosen,Approved=approved,Kind=kind,CreatedAt=now.ToString("o")};
 }
 public BuiltinDay Read(Settings s,DateTime now){lock(gate)return ReadDay(s,now);}
 public BuiltinDay Prepare(Settings s,DateTime now,bool approve=true){lock(gate){
  var day=ReadDay(s,now);var plan=RitualSchedule.Preparation(s,now);var wanted=new HashSet<string>(plan.Select(slot=>slot.Id),StringComparer.Ordinal);bool changed=!File.Exists(PathFor(s,now));
  foreach(var row in day.Rows.Where(row=>row.Kind=="planned")){bool active=wanted.Contains(row.Slot)&&!row.Rejected&&!row.Deleted;if(row.Active!=active){row.Active=active;changed=true;}}
  var missing=plan.Where(slot=>!day.Rows.Any(row=>row.Kind=="planned"&&row.Slot==slot.Id&&!row.Rejected&&!row.Deleted)).ToArray();
  if(missing.Length>0){var history=History();var today=new HashSet<string>(day.Rows.Select(row=>row.Text),StringComparer.Ordinal);foreach(var slot in missing)day.Rows.Add(New(day,slot.Id,slot.Category,slot.Followup,approve,"planned",now,history,today));changed=true;}
  if(changed)Write(s,now,day);return day;
 }}
 public BuiltinPhrase Consume(Settings s,DateTime now,string category,bool followup,bool fallback=true){lock(gate){
  if(!Categories.Contains(category))throw new ArgumentException(AppText.T("Неизвестная категория фразы","Unknown phrase category"));var day=ReadDay(s,now);
  var row=day.Rows.FirstOrDefault(item=>item.Active&&item.Approved&&!item.Rejected&&!item.Deleted&&item.Category==category&&item.Followup==followup&&string.IsNullOrEmpty(item.UsedAt));
  if(row==null){if(!fallback)return null;row=New(day,"fallback:"+Guid.NewGuid().ToString("N"),category,followup,true,"fallback",now,History(),new HashSet<string>(day.Rows.Select(item=>item.Text),StringComparer.Ordinal));day.Rows.Add(row);}
  row.UsedAt=now.ToString("o");Write(s,now,day);return row;
 }}
 public BuiltinDay Add(Settings s,DateTime now,string category,int count,bool followup=false){lock(gate){
  if(!Categories.Contains(category)||count<1||count>100)throw new ArgumentException(AppText.T("Неверная партия фраз","Invalid phrase batch"));var day=ReadDay(s,now);var history=History();var today=new HashSet<string>(day.Rows.Select(row=>row.Text),StringComparer.Ordinal);
  for(int i=0;i<count;i++)day.Rows.Add(New(day,"manual:"+Guid.NewGuid().ToString("N"),category,followup,false,"manual",now,history,today));Write(s,now,day);return day;
 }}
 public void Select(Settings s,DateTime now,string[] ids,string action){lock(gate){
  if(ids==null||ids.Length>1000||!new[]{"keep","exclude","delete"}.Contains(action))throw new ArgumentException(AppText.T("Неверный отбор фраз","Invalid phrase selection"));var day=ReadDay(s,now);var selected=new HashSet<string>(ids,StringComparer.Ordinal);
  foreach(var row in day.Rows.Where(row=>selected.Contains(row.Id))){if(action=="delete"){row.Deleted=true;row.Active=false;}else{row.Approved=action=="keep";row.Rejected=action=="exclude";row.Active=action=="keep";}}
  Write(s,now,day);
 }}
 public string Export(Settings s,DateTime now,string[] ids){lock(gate){var day=ReadDay(s,now);var selected=ids==null||ids.Length==0?null:new HashSet<string>(ids,StringComparer.Ordinal);return json.Serialize(new{day=day.Day,language=day.Language,rows=day.Rows.Where(row=>!row.Deleted&&(selected==null||selected.Contains(row.Id))).ToArray()});}}
 public void Remember(Settings s,DateTime now,string text,string category,bool followup){lock(gate){
  if(string.IsNullOrWhiteSpace(text)||text.Length>400||!Categories.Contains(category))throw new ArgumentException(AppText.T("Неверная проигранная фраза","Invalid played phrase"));var day=ReadDay(s,now);var matches=day.Rows.Where(row=>row.Text==text).ToArray();
  if(matches.Length==0)day.Rows.Add(new BuiltinPhrase{Id="stock-"+Guid.NewGuid().ToString("N"),Slot="external:"+Guid.NewGuid().ToString("N"),Text=text,Category=category,Followup=followup,Kind="external",CreatedAt=now.ToString("o"),UsedAt=now.ToString("o")});
  else foreach(var row in matches)if(string.IsNullOrEmpty(row.UsedAt))row.UsedAt=now.ToString("o");Write(s,now,day);
 }}
}
