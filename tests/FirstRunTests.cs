using System;
using System.Collections.Generic;
using System.Drawing;
using System.IO;
using System.Reflection;
using System.Linq;
using System.Speech.Synthesis;
using System.Threading.Tasks;
using System.Web.Script.Serialization;
using System.Windows.Forms;

// Deliberately omits --self-test/--preview: the real Shown/DesktopStartup path,
// mascot, timers, hooks and fallback reminders run. --data-dir isolates storage.
public static class FirstRunTests {
 static readonly BindingFlags Private=BindingFlags.Instance|BindingFlags.NonPublic;
 static readonly List<string> checks=new List<string>();
 static T Get<T>(object owner,string key){return (T)owner.GetType().GetField(key,Private).GetValue(owner);}
 static void Set(object owner,string key,object value){owner.GetType().GetField(key,Private).SetValue(owner,value);}
 static object Call(object owner,string key,params object[] args){return owner.GetType().GetMethod(key,Private).Invoke(owner,args);}
 static void Check(bool yes,string name){if(!yes)throw new Exception(name);checks.Add(name);Console.WriteLine("PASS "+name);}
 [STAThread] public static int Main(string[] args){
  int at=Array.IndexOf(args,"--data-dir");if(at<0||at+1>=args.Length||Array.IndexOf(args,"--first-run-probe")<0)return 2;
  string data=Path.GetFullPath(args[at+1]);bool returning=Array.IndexOf(args,"--expect-ready")>=0;string report=Path.Combine(data,returning?"returning.json":"first-run.json");
  Application.EnableVisualStyles();Application.SetCompatibleTextRenderingDefault(false);bool startupBefore=StartupIntegration.IsEnabled(StartupIntegration.Folder);int code=1;
  string shownText="";
  using(var form=new MainForm())using(var monitor=new Timer{Interval=100}){
   var settings=Get<Settings>(form,"settings");settings.Voice=false;settings.NotificationSound=false;settings.AutomaticUpdates=false;
   DateTime deadline=DateTime.UtcNow.AddSeconds(20);bool checking=false;
   monitor.Tick+=async delegate{if(checking)return;if((!Get<bool>(form,"desktopReady")||Get<bool>(form,"preparationBusy"))&&DateTime.UtcNow<deadline)return;checking=true;monitor.Stop();try{
    Check(Get<bool>(form,"desktopReady"),"normal startup completes without Python or models");
    Check(Directory.Exists(Path.Combine(data,"builtin-days"))&&Directory.GetFiles(Path.Combine(data,"builtin-days"),"*.json").Length>0,"normal startup prepares a persisted basic day list");
    Check(!form.Visible&&Get<PetForm>(form,"pet").Visible,"normal launch shows the mascot and keeps the advanced host hidden");
    Check(string.IsNullOrEmpty(Get<PetForm>(form,"pet").RenderingError)&&Get<AlphaSurface>(Get<PetForm>(form,"pet"),"surface")!=null,"normal startup presents the mascot through a real alpha surface");
    Check(Get<Timer>(form,"timer").Enabled&&Get<Timer>(form,"dailyTimer").Enabled,"schedule and day preparation timers are active");
    Check(Get<NativeActivity>(form,"activity").Available,"real mouse and keyboard hooks are available");
    var simple=Get<SimpleSettingsForm>(form,"simpleSettings");
    Check(returning?simple==null:simple!=null&&simple.Visible,"setup appears only before initial preferences are saved");
    Check(Get<System.Diagnostics.Process>(form,"backend")==null&&!Directory.Exists(Path.Combine(data,"runtime"))&&!Directory.Exists(Path.Combine(data,"models"))&&!File.Exists(Path.Combine(data,"library.sqlite3")),"basic use creates no backend, runtime, weights or library database");
    Check(settings.CharacterFile==""&&settings.VoiceProfile=="","isolated first launch does not import development personal presets");
    Check(!Get<bool>(form,"updateChecking")&&Get<DateTime>(form,"nextUpdateCheckUtc")==DateTime.MinValue,"disabled update checking is honored on startup");
    if(!returning){
     using(var image=new Bitmap(simple.Width,simple.Height)){simple.DrawToBitmap(image,new Rectangle(0,0,image.Width,image.Height));image.Save(Path.Combine(data,"first-run.png"));}
     Check((bool)Call(simple,"SaveDraft",true),"initial settings can be completed without installing components");simple.Close();
    }else Check(settings.SetupComplete,"completed setup survives a new process");
    await (Task)Call(form,"StartDesktopReminder","exercise",false);var message=Get<PetMessage>(form,"petMessage");
    Check(message!=null&&message.Visible&&Get<bool>(form,"messagePending")&&!string.IsNullOrWhiteSpace(Get<TextBox>(form,"lastText").Text),"a ready-made reminder is shown with no models");
    shownText=Get<TextBox>(form,"lastText").Text;if(returning){var first=new JavaScriptSerializer().Deserialize<Dictionary<string,object>>(File.ReadAllText(Path.Combine(data,"first-run.json")));Check(shownText!=Convert.ToString(first["shown_text"]),"a new application process does not repeat the previous basic reminder");}
    Get<StudioButton>(message,"accept").PerformClick();Check(!message.Visible&&!Get<bool>(form,"messagePending"),"the reminder remains actionable and confirmation hides it");
    Call(form,"DesktopBreakfast");Check(settings.ExerciseUtc.HasValue&&settings.ExerciseUtc.Value>DateTime.UtcNow.AddMinutes(settings.ExerciseAfter-1),"breakfast starts the independent exercise timer in basic mode");
    using(var inventory=new SpeechSynthesizer()){
     var languages=inventory.GetInstalledVoices().Where(voice=>voice.Enabled).Select(voice=>voice.VoiceInfo.Culture.TwoLetterISOLanguageName).ToArray();string missing=new[]{"ru","en"}.FirstOrDefault(language=>!languages.Contains(language));
     if(missing!=null){string previousLanguage=settings.SpeechLanguage;settings.SpeechLanguage=missing;settings.Voice=true;
      await (Task)Call(form,"StartDesktopReminder","exercise",false);
      Check(Get<object>(form,"activeSystemPrompt")==null&&Get<string>(form,"preparationStatus").StartsWith(L.T("В Windows нет голоса","Windows has no voice")),"an unavailable language voice produces a clear notice instead of reading in another language");
      settings.SpeechLanguage=previousLanguage;settings.Voice=false;Call(form,"Save");Get<StudioButton>(Get<PetMessage>(form,"petMessage"),"accept").PerformClick();
     }
    }
    Check(StartupIntegration.IsEnabled(StartupIntegration.Folder)==startupBefore,"isolated startup leaves the real Windows startup entry unchanged");
    File.WriteAllText(report,new JavaScriptSerializer().Serialize(new{ok=true,checks=checks,shown_text=shownText}));code=0;
   }catch(Exception error){Console.Error.WriteLine(error);File.WriteAllText(report,error.ToString());}finally{Set(form,"exiting",true);form.Close();}};
   form.Shown+=delegate{monitor.Start();};Application.Run(form);
  }
  Console.WriteLine("PASS "+checks.Count+" first-run checks");return code;
 }
}
