using System;
using System.Collections.Generic;
using System.IO;
using System.Net;
using System.Net.Http;
using System.Reflection;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Web.Script.Serialization;
using System.Windows.Forms;

// Drive the real reminder controller against an in-memory HTTP handler. No
// hooks, models, speech output or network sockets are used by this executable.
public static class ReminderUiTests {
 static readonly BindingFlags Private=BindingFlags.NonPublic|BindingFlags.Instance;
 static int passed;
 static T Get<T>(object owner,string name){return (T)owner.GetType().GetField(name,Private).GetValue(owner);}
 static void Set(object owner,string name,object value){owner.GetType().GetField(name,Private).SetValue(owner,value);}
 static object Call(object owner,string name,params object[] args){return owner.GetType().GetMethod(name,Private).Invoke(owner,args);}
 static void Check(bool ok,string name){if(!ok)throw new Exception(name);Console.WriteLine("PASS "+name);passed++;}
 sealed class LocalHandler:HttpMessageHandler {
  readonly JavaScriptSerializer json=new JavaScriptSerializer();
  readonly int delayMs;
  DateTime started;
  public int RuntimeCalls,Claims,Consumes,SettingsReadDelayMs;
  public Dictionary<string,object> Preferences=new Dictionary<string,object>{{"text_model","text-qwen35-4b"},{"runtime_text_model","text-qvikhr3-8b"},{"tts_model","tts-1.7b"},{"runtime",true},{"swear",true},{"auto_daily",false},{"daily_auto_keep",false},{"batch_count",13},{"device","auto"},{"delivery_catalog",new object[0]},{"cosy_style","auto"},{"character_description",""},{"categories",new object[]{"break"}}};
  public LocalHandler(int delay){delayMs=delay;}
  object Row(string text){return new{id="fresh",text=text,audio_path=(string)null};}
  object Job(){bool done=(DateTime.UtcNow-started).TotalMilliseconds>=delayMs;return new{id="runtime-job",state=done?"done":"running",message="Preparing",done=0,total=0,result=done?Row("Fresh generated reminder."):null};}
  protected override async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request,CancellationToken cancel){
   object result;string path=request.RequestUri.AbsolutePath;
   if(path=="/settings"){
    if(request.Method==HttpMethod.Post){var update=(Dictionary<string,object>)json.DeserializeObject(await request.Content.ReadAsStringAsync());foreach(var entry in update)Preferences[entry.Key]=entry.Value;}
    result=new Dictionary<string,object>(Preferences);
    if(request.Method==HttpMethod.Get&&SettingsReadDelayMs>0)await Task.Delay(SettingsReadDelayMs,cancel);
   }
   else if(path=="/jobs"){RuntimeCalls++;started=DateTime.UtcNow;result=Job();}
   else if(path=="/jobs/runtime-job")result=Job();
   else if(path=="/claim"){Claims++;result=new{row=Row("Fresh generated reminder.")};}
   else if(path=="/consume"){Consumes++;result=new{row=Row("Prepared cached reminder.")};}
   else if(path=="/library")result=new{rows=new object[0]};
   else if(path=="/status")result=new{components=new object[0],stats=new{audio_bytes=0}};
   else throw new Exception("Unexpected test route: "+path);
   return new HttpResponseMessage(HttpStatusCode.OK){Content=new StringContent(json.Serialize(result),Encoding.UTF8,"application/json")};
  }
 }
 static async Task Drain(MainForm form){var until=DateTime.UtcNow.AddSeconds(20);while(Get<HashSet<string>>(form,"activeJobs").Count>0){if(DateTime.UtcNow>until)throw new TimeoutException("Test runtime job did not finish");await Task.Delay(100);}}
 static MainForm Subject(string folder,LocalHandler handler){
  Directory.CreateDirectory(Path.Combine(folder,"runtime"));File.WriteAllText(Path.Combine(folder,"runtime","python.exe"),"not executed by this test");var form=new MainForm();var settings=Get<Settings>(form,"settings");settings.VoiceProvider="System";settings.TextGeneration=true;settings.RuntimeGeneration=true;settings.SpeechLanguage="en";settings.Voice=false;settings.NotificationSound=false;settings.Paused=false;settings.DeferGenerationFullscreen=false;
  settings.BackendSettingsMigrated=true;settings.BackendPersonalizationMigrated=true;settings.BackendDailySettingsMigrated=true;settings.AutomaticPreparation=false;settings.AutoKeepDaily=false;
  Set(form,"backendStarting",Task.FromResult(0));Set(form,"endpoint","http://127.0.0.1:1");Set(form,"localHttp",new HttpClient(handler));return form;
 }
 static void Close(MainForm form){Set(form,"exiting",true);var message=Get<PetMessage>(form,"petMessage");if(message!=null)message.Close();Call(form,"CloseLocalAi");Get<NotifyIcon>(form,"tray").Dispose();Get<System.Windows.Forms.Timer>(form,"timer").Dispose();form.Dispose();}
 static async Task Run(string folder){
  var fast=new LocalHandler(0);var form=Subject(folder,fast);try{await (Task)Call(form,"SpeakDesktopReminder","break",false);Check(Get<TextBox>(form,"lastText").Text=="Fresh generated reminder."&&fast.RuntimeCalls==1&&fast.Claims==1&&fast.Consumes==0,"fast runtime uses fresh result without consuming a spare recording");}finally{Close(form);}
  var slow=new LocalHandler(10000);form=Subject(folder,slow);try{await (Task)Call(form,"SpeakDesktopReminder","break",true);Check(Get<TextBox>(form,"lastText").Text=="Prepared cached reminder."&&slow.Consumes==1&&slow.Claims==0,"slow runtime uses a prepared fallback within its wait budget");await Drain(form);Check(Get<TextBox>(form,"lastText").Text=="Prepared cached reminder."&&slow.Claims==0,"late runtime neither replaces the shown message nor claims its unused result");}finally{Close(form);}
  var acknowledge=new LocalHandler(2000);form=Subject(folder,acknowledge);try{var speaking=(Task)Call(form,"SpeakDesktopReminder","exercise",false);await Task.Delay(100);var message=Get<PetMessage>(form,"petMessage");message.Hide();message.Confirmed();await speaking;await Drain(form);Check(acknowledge.Claims==0&&!Get<bool>(form,"messagePending"),"acknowledgement prevents a late runtime result from reviving the message");}finally{Close(form);}
  var cached=new LocalHandler(0);form=Subject(folder,cached);try{Get<Settings>(form,"settings").RuntimeGeneration=false;await (Task)Call(form,"SpeakDesktopReminder","break",false);Check(cached.RuntimeCalls==0&&cached.Consumes==1&&Get<TextBox>(form,"lastText").Text=="Prepared cached reminder.","disabled runtime consumes prepared phrases without starting generation");}finally{Close(form);}
  var paused=new LocalHandler(0);form=Subject(folder,paused);try{
   Get<Settings>(form,"settings").Paused=true;await (Task)Call(form,"StartDesktopReminder","exercise",true);
   Check(paused.RuntimeCalls==0&&Get<PetMessage>(form,"petMessage")==null,"pause suppresses an automatic reminder");
   await (Task)Call(form,"StartDesktopReminder","exercise",false);
   Check(paused.Claims==1&&Get<bool>(form,"messagePending")&&Get<bool>(form,"manualMessage")&&Get<Settings>(form,"settings").Paused,"manual stretch works during pause without resuming automatic reminders");
  }finally{Close(form);}
  var pauseDuringRuntime=new LocalHandler(2000);form=Subject(folder,pauseDuringRuntime);try{
   var speaking=(Task)Call(form,"StartDesktopReminder","exercise",false);await Task.Delay(100);Call(form,"SetDesktopPause",15);await speaking;await Drain(form);
   Check(pauseDuringRuntime.Claims==0&&!Get<bool>(form,"messagePending")&&!Get<bool>(form,"manualMessage"),"entering pause cancels an in-flight manual reminder");
  }finally{Close(form);}
  var disabledDuringRuntime=new LocalHandler(2000);form=Subject(folder,disabledDuringRuntime);try{
   Get<Settings>(form,"settings").ExerciseEnabled=true;var speaking=(Task)Call(form,"StartDesktopReminder","exercise",true);await Task.Delay(100);Call(form,"SetDesktopExerciseEnabled",false);await speaking;await Drain(form);
   Check(disabledDuringRuntime.Claims==0&&!Get<bool>(form,"messagePending")&&Get<ReminderCycle>(form,"cycle")==null,"disabling stretch cancels its in-flight voice and prevents a late runtime result reviving it");
  }finally{Close(form);}
  var migration=new LocalHandler(0);form=Subject(folder,migration);try{
   var settings=Get<Settings>(form,"settings");settings.BackendSettingsMigrated=false;settings.BackendDailySettingsMigrated=false;settings.AutomaticPreparation=true;settings.AutoKeepDaily=true;settings.Swear=false;settings.RuntimeGeneration=false;
   await (Task)Call(form,"LoadLocalSettings");
   Check(settings.BackendSettingsMigrated&&settings.TextModel=="text-qwen35-4b"&&settings.RuntimeTextModel=="text-qvikhr3-8b"&&settings.VoiceModel=="tts-1.7b"&&settings.Swear&&settings.RuntimeGeneration,"legacy database model choices migrate once");
   migration.Preferences["text_model"]="text-saiga-llama3-8b";migration.Preferences["runtime"]=false;migration.Preferences["swear"]=false;
   await (Task)Call(form,"LoadLocalSettings");
   Check(settings.TextModel=="text-qwen35-4b"&&settings.RuntimeGeneration&&settings.Swear&&Get<CheckBox>(form,"runtimeToggle").Checked&&Get<CheckBox>(form,"localSwear").Checked,"later database reloads do not overwrite native settings or advanced selectors");
   Check(!Get<CheckBox>(form,"autoDaily").Checked&&!Get<CheckBox>(form,"autoKeep").Checked&&!settings.AutomaticPreparation&&!settings.AutoKeepDaily&&settings.BackendDailySettingsMigrated,"legacy daily preparation choices migrate into shared controls");
  }finally{Close(form);}
  var race=new LocalHandler(0){SettingsReadDelayMs=300};form=Subject(folder,race);try{
   var settings=Get<Settings>(form,"settings");settings.BackendSettingsMigrated=false;
   var loading=(Task)Call(form,"LoadLocalSettings");
   var draft=new JavaScriptSerializer().Deserialize<Settings>(new JavaScriptSerializer().Serialize(settings));draft.TextModel="text-gemma4-e4b";draft.RuntimeTextModel="text-qwen35-9b";draft.VoiceModel="tts-0.6b";draft.RuntimeGeneration=false;draft.Swear=false;
   Call(form,"SaveDesktopSettings",draft);await loading;await (Task)Call(form,"SyncDesktopSettings");
   Check(settings.TextModel==draft.TextModel&&settings.RuntimeTextModel==draft.RuntimeTextModel&&settings.VoiceModel==draft.VoiceModel&&!settings.RuntimeGeneration&&!settings.Swear,"a native save during startup wins over an older database response");
   Check(Convert.ToString(race.Preferences["text_model"])==draft.TextModel&&Convert.ToString(race.Preferences["tts_model"])==draft.VoiceModel&&Get<ComboBox>(form,"textChoice").SelectedIndex==1,"native choices synchronize to the backend and advanced model controls");
  }finally{Close(form);}
  var syncFirst=new LocalHandler(0);form=Subject(folder,syncFirst);try{
   var settings=Get<Settings>(form,"settings");settings.BackendSettingsMigrated=false;settings.BackendSettingsOverrides=new List<string>();settings.TextModel="text-gemma4-12b";
   await (Task)Call(form,"SyncDesktopSettings");
   Check(settings.TextModel=="text-qwen35-4b"&&Convert.ToString(syncFirst.Preferences["text_model"])=="text-qwen35-4b"&&Get<ComboBox>(form,"textChoice").SelectedIndex==2,"synchronization imports previous choices before its first write and refreshes advanced controls");
  }finally{Close(form);}
  var partial=new LocalHandler(0);form=Subject(folder,partial);try{
   var settings=Get<Settings>(form,"settings");settings.BackendSettingsMigrated=false;settings.BackendSettingsOverrides=new List<string>();settings.TextModel="text-gemma4-12b";settings.RuntimeTextModel="text-gemma4-12b";settings.VoiceModel="tts-cosyvoice3";
   Set(form,"endpoint",null);
   var draft=new JavaScriptSerializer().Deserialize<Settings>(new JavaScriptSerializer().Serialize(settings));draft.VoiceModel="tts-0.6b";
   Call(form,"SaveDesktopSettings",draft);
   var saved=new JavaScriptSerializer().Deserialize<Settings>(File.ReadAllText(Path.Combine(folder,"settings.json")));
   Check(!saved.BackendSettingsMigrated&&saved.BackendSettingsOverrides.Contains("tts_model"),"a choice made before migration is persisted for the next startup");
   Set(form,"endpoint","http://127.0.0.1:1");await (Task)Call(form,"LoadLocalSettings");
   Check(settings.VoiceModel=="tts-0.6b"&&settings.TextModel=="text-qwen35-4b"&&settings.RuntimeTextModel=="text-qvikhr3-8b"&&settings.BackendSettingsMigrated&&settings.BackendSettingsOverrides.Count==0,"changing only the voice before migration still preserves old text model choices");
  }finally{Close(form);}
  var saveFailure=new LocalHandler(0);form=Subject(folder,saveFailure);try{
   var settings=Get<Settings>(form,"settings");settings.UserName="Original name";Call(form,"Save");byte[] previous=File.ReadAllBytes(Path.Combine(folder,"settings.json"));
   var draft=new JavaScriptSerializer().Deserialize<Settings>(new JavaScriptSerializer().Serialize(settings));draft.UserName="Unsaved name";draft.RuntimeGeneration=!settings.RuntimeGeneration;
   string occupied=Path.Combine(folder,"blocked-settings.json");Directory.CreateDirectory(occupied);Set(form,"config",occupied);bool failed=false;
   try{Call(form,"SaveDesktopSettings",draft);}catch(TargetInvocationException error){failed=error.InnerException is IOException;}
   Check(failed&&settings.UserName=="Original name"&&settings.RuntimeGeneration!=draft.RuntimeGeneration,"failed native save reports failure and restores live settings");
   Check(Convert.ToBase64String(previous)==Convert.ToBase64String(File.ReadAllBytes(Path.Combine(folder,"settings.json")))&&Directory.GetFiles(folder,"blocked-settings.json.tmp-*").Length==0,"failed native save preserves the last settings file and removes temporary data");
  }finally{Close(form);}
  var planning=new LocalHandler(0);form=Subject(folder,planning);try{
   var settings=Get<Settings>(form,"settings");settings.BackendDailySettingsMigrated=false;settings.BackendSettingsOverrides=new List<string>();settings.AutomaticPreparation=true;settings.AutoKeepDaily=true;
   await (Task)Call(form,"LoadLocalSettings");planning.Preferences["auto_daily"]=true;planning.Preferences["daily_auto_keep"]=true;await (Task)Call(form,"LoadLocalSettings");
   Check(!settings.AutomaticPreparation&&!settings.AutoKeepDaily,"migrated daily choices are not overwritten by later database reloads");
   settings.BackendDailySettingsMigrated=false;settings.BackendSettingsOverrides=new List<string>{"auto_daily","daily_auto_keep"};await (Task)Call(form,"SyncDesktopSettings");
   Check(!settings.AutomaticPreparation&&!settings.AutoKeepDaily&&!Convert.ToBoolean(planning.Preferences["auto_daily"])&&!Convert.ToBoolean(planning.Preferences["daily_auto_keep"]),"native daily choices made before migration win over stale database defaults");
  }finally{Close(form);}
 }
 [STAThread] public static int Main(string[] args){int at=Array.IndexOf(args,"--data-dir");if(at<0||!Array.Exists(args,a=>a=="--self-test")){Console.Error.WriteLine("Pass --self-test --data-dir <isolated work fixture>");return 2;}string folder=Path.GetFullPath(args[at+1]);Application.EnableVisualStyles();Application.SetCompatibleTextRenderingDefault(false);int code=1;using(var host=new Form{Opacity=0,ShowInTaskbar=false,Width=1,Height=1}){host.Shown+=async delegate{try{await Run(folder);Console.WriteLine("PASS "+passed+" reminder UI checks");code=0;}catch(Exception e){Console.Error.WriteLine(e);}finally{host.Close();}};Application.Run(host);}return code;}
}
