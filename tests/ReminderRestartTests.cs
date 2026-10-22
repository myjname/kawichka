using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Speech.Synthesis;
using System.Threading.Tasks;
using System.Web.Script.Serialization;
using System.Windows.Forms;

// Four ordinary app processes exercise Shown/DesktopStartup/FormClosing.
// Only the specified fixture is read/written; no models or speech are used.
public static class ReminderRestartTests {
 static readonly BindingFlags Private=BindingFlags.Instance|BindingFlags.NonPublic;
 static readonly JavaScriptSerializer Json=new JavaScriptSerializer();
 static int passed;
 static T Get<T>(object owner,string name){return (T)owner.GetType().GetField(name,Private).GetValue(owner);}
 static void Set(object owner,string name,object value){owner.GetType().GetField(name,Private).SetValue(owner,value);}
 static object Call(object owner,string name,params object[] args){return owner.GetType().GetMethod(name,Private).Invoke(owner,args);}
 static void Check(bool yes,string name){if(!yes)throw new Exception(name);Console.WriteLine("PASS "+name);passed++;}
 static int Used(string root){string folder=Path.Combine(root,"builtin-days");return Directory.GetFiles(folder,"*.json").Sum(file=>Json.Deserialize<BuiltinDay>(File.ReadAllText(file)).Rows.Count(row=>row.UsedAt.Length>0));}
 [STAThread]public static int Main(string[] args){
  int at=Array.IndexOf(args,"--data-dir"),stageAt=Array.IndexOf(args,"--stage");if(at<0||stageAt<0||!args.Contains("--restart-probe"))return 2;
  string root=Path.GetFullPath(args[at+1]),stage=args[stageAt+1],witness=Path.Combine(root,"restart-witness.json");int code=1;
  Application.EnableVisualStyles();Application.SetCompatibleTextRenderingDefault(false);
  using(var form=new MainForm())using(var monitor=new Timer{Interval=100}){
   var settings=Get<Settings>(form,"settings");settings.SetupComplete=true;settings.AutomaticUpdates=false;settings.Voice=false;settings.NotificationSound=false;settings.TextGeneration=false;settings.VoiceProvider="System";settings.Paused=false;settings.WorkDays=new int[0];settings.MealsEnabled=false;settings.RepeatMinutes=15;settings.TimeZoneId="UTC";settings.TrustConfirmation=false;
   // Simulate an actively working user without sending keyboard/mouse events.
   var activity=Get<NativeActivity>(form,"activity");if(activity!=null)Set(Get<ActivityFilter>(activity,"filter"),"lastSignificant",DateTime.UtcNow);
   DateTime deadline=DateTime.UtcNow.AddSeconds(20);bool checking=false;
   monitor.Tick+=async delegate{if(checking)return;if((!Get<bool>(form,"desktopReady")||Get<bool>(form,"preparationBusy"))&&DateTime.UtcNow<deadline)return;checking=true;monitor.Stop();try{
    Check(Get<bool>(form,"desktopReady"),"ordinary startup completed for "+stage);
    Check(Get<System.Diagnostics.Process>(form,"backend")==null&&!Directory.Exists(Path.Combine(root,"runtime")),"restart restoration does not require Python");
    if(stage=="create"){
     await (Task)Call(form,"StartDesktopReminder","exercise",true);var cycle=Get<ReminderCycle>(form,"cycle");cycle.RepeatsSent=1;
     Call(form,"PersistDesktopReminder",DateTime.UtcNow);var text=Get<TextBox>(form,"lastText").Text;
     Check(Get<bool>(form,"messagePending")&&Get<PetMessage>(form,"petMessage").Visible,"first process displays an unacknowledged reminder");
     File.WriteAllText(witness,Json.Serialize(new{text=text,deadline=cycle.NextRepeatUtc.ToString("o"),used=Used(root)}));
    }else{
     var previous=Json.Deserialize<Dictionary<string,object>>(File.ReadAllText(witness));
     if(stage=="restore"){
      Check(Get<bool>(form,"messagePending")&&Get<PetMessage>(form,"petMessage").Visible&&Get<TextBox>(form,"lastText").Text==Convert.ToString(previous["text"]),"second process restores the original message through normal startup");
      Check(Get<ReminderCycle>(form,"cycle").RepeatsSent==1&&Math.Abs((Get<ReminderCycle>(form,"cycle").NextRepeatUtc-DateTime.Parse(Convert.ToString(previous["deadline"]),CultureInfo.InvariantCulture,DateTimeStyles.RoundtripKind)).TotalMilliseconds)<1,"second process preserves the count and deadline");
      Check(Get<SpeechSynthesizer>(form,"systemSpeech")==null&&Used(root)==Convert.ToInt32(previous["used"]),"startup restoration is silent and does not reserve a second phrase");
      Get<StudioButton>(Get<PetMessage>(form,"petMessage"),"accept").PerformClick();previous["deadline"]=Get<ReminderCycle>(form,"cycle").NextRepeatUtc.ToString("o");File.WriteAllText(witness,Json.Serialize(previous));
     }else if(stage=="hidden"){
      Check(!Get<bool>(form,"messagePending")&&Get<PetMessage>(form,"petMessage")==null&&Get<ReminderCycle>(form,"cycle").HiddenByConfirmation&&!Get<ReminderCycle>(form,"cycle").Finished,"third process retains untrusted acknowledgement without reopening its message");
      Check(Get<ReminderCycle>(form,"cycle").RepeatsSent==1&&Math.Abs((Get<ReminderCycle>(form,"cycle").NextRepeatUtc-DateTime.Parse(Convert.ToString(previous["deadline"]),CultureInfo.InvariantCulture,DateTimeStyles.RoundtripKind)).TotalMilliseconds)<1,"confirmation does not reset the repeat count after restart");
      settings.TrustConfirmation=true;await (Task)Call(form,"StartDesktopReminder","exercise",true);Get<StudioButton>(Get<PetMessage>(form,"petMessage"),"accept").PerformClick();
      Check(ReminderJournal.Read(Path.Combine(root,"reminder-state.json"),DateTime.UtcNow)==null,"trusted confirmation clears state before the process exits");
     }else if(stage=="cleared")Check(!Get<bool>(form,"messagePending")&&Get<ReminderCycle>(form,"cycle")==null&&Get<PetMessage>(form,"petMessage")==null,"fourth process does not revive a completed reminder");
     else throw new ArgumentException("Unknown stage");
    }
    code=0;Console.WriteLine("PASS "+passed+" ordinary restart checks ("+stage+")");
   }catch(Exception error){Console.Error.WriteLine(error);File.WriteAllText(Path.Combine(root,stage+"-error.txt"),error.ToString());}finally{Set(form,"exiting",true);form.Close();}};
   form.Shown+=delegate{monitor.Start();};Application.Run(form);
  }return code;
 }
}
