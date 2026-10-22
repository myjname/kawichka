using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Threading.Tasks;
using System.Windows.Forms;
using Microsoft.Win32;

public static class ReminderStateUiTests {
 static readonly BindingFlags Private=BindingFlags.Instance|BindingFlags.NonPublic;
 static int passed;
 static T Get<T>(object owner,string field){return (T)owner.GetType().GetField(field,Private).GetValue(owner);}
 static void Set(object owner,string field,object value){owner.GetType().GetField(field,Private).SetValue(owner,value);}
 static object Call(object owner,string method,params object[] args){return owner.GetType().GetMethod(method,Private).Invoke(owner,args);}
 static void Check(bool yes,string name){if(!yes)throw new Exception(name);Console.WriteLine("PASS "+name);passed++;}
 static bool SameTime(DateTime a,DateTime b){return Math.Abs((a-b).TotalMilliseconds)<1;}
 static MainForm Subject(){var form=new MainForm();var settings=Get<Settings>(form,"settings");settings.TimeZoneId="UTC";settings.WorkDays=new[]{0,1,2,3,4,5,6};settings.WorkStart=0;settings.WorkEnd=1439;settings.Paused=false;settings.Voice=false;settings.NotificationSound=false;settings.TextGeneration=false;settings.VoiceProvider="System";settings.TrustConfirmation=false;settings.RepeatMinutes=15;settings.AutomaticUpdates=false;return form;}
 static void Close(MainForm form){Set(form,"exiting",true);var message=Get<PetMessage>(form,"petMessage");if(message!=null)message.Dispose();Call(form,"CloseLocalAi");Get<NotifyIcon>(form,"tray").Dispose();Get<System.Windows.Forms.Timer>(form,"timer").Dispose();form.Dispose();}
 static ReminderCheckpoint Read(string root){return ReminderJournal.Read(Path.Combine(root,"reminder-state.json"),DateTime.UtcNow);}
 static async Task Run(string root){
  DateTime deadline;string original;int used;
  var form=Subject();try{
   await (Task)Call(form,"StartDesktopReminder","exercise",true);var cycle=Get<ReminderCycle>(form,"cycle");cycle.RepeatsSent=1;deadline=cycle.NextRepeatUtc;original=Get<TextBox>(form,"lastText").Text;Call(form,"PersistDesktopReminder",DateTime.UtcNow);
   Check(Read(root).Pending&&Read(root).Cycle.RepeatsSent==1,"a displayed automatic reminder is journaled with its repeat count");
   Get<TextBox>(form,"lastText").Text="Unrelated advanced panel text";Call(form,"PersistDesktopReminder",DateTime.UtcNow);Check(Read(root).Text==original,"journal text comes from the displayed bubble rather than an unrelated UI field");
   used=new BuiltinDays(root).Read(Get<Settings>(form,"settings"),DateTime.UtcNow).Rows.Count(row=>row.UsedAt.Length>0);
  }finally{Close(form);}
  form=Subject();try{
   int version=Get<int>(form,"audioVersion");Call(form,"RestoreDesktopReminder",DateTime.UtcNow);
   Check(Get<bool>(form,"messagePending")&&Get<PetMessage>(form,"petMessage").Visible&&Get<TextBox>(form,"lastText").Text==original,"a quick restart restores the original unacknowledged message");
   var cycle=Get<ReminderCycle>(form,"cycle");Check(cycle.RepeatsSent==1&&SameTime(cycle.NextRepeatUtc,deadline)&&Get<int>(form,"audioVersion")==version&&Get<object>(form,"activeSystemPrompt")==null,"restoration keeps the deadline and does not replay speech or chime");
   Check(new BuiltinDays(root).Read(Get<Settings>(form,"settings"),DateTime.UtcNow).Rows.Count(row=>row.UsedAt.Length>0)==used&&!Directory.Exists(Path.Combine(root,"runtime")),"restoration neither consumes another phrase nor starts Python");
   Get<StudioButton>(Get<PetMessage>(form,"petMessage"),"accept").PerformClick();deadline=cycle.NextRepeatUtc;
   Check(!Read(root).Pending&&Read(root).Cycle.HiddenByConfirmation&&!Read(root).Cycle.Finished,"acknowledgement is saved without trusting it to end the activity check");
  }finally{Close(form);}
  form=Subject();try{
   Call(form,"RestoreDesktopReminder",DateTime.UtcNow);Check(!Get<bool>(form,"messagePending")&&Get<PetMessage>(form,"petMessage")==null&&SameTime(Get<ReminderCycle>(form,"cycle").NextRepeatUtc,deadline),"acknowledged message stays hidden after another restart");
   Get<Settings>(form,"settings").TrustConfirmation=true;await (Task)Call(form,"StartDesktopReminder","exercise",true);Get<StudioButton>(Get<PetMessage>(form,"petMessage"),"accept").PerformClick();Check(Read(root)==null,"trusted acknowledgement durably clears the completed reminder");
   Get<Settings>(form,"settings").TrustConfirmation=false;await (Task)Call(form,"StartDesktopReminder","exercise",true);Call(form,"SetDesktopPause",15);Check(Read(root)==null,"entering pause clears the saved chain and message");
  }finally{Close(form);}
  form=Subject();try{
   var settings=Get<Settings>(form,"settings");var now=DateTime.UtcNow;
   var state=new ReminderCheckpoint{Category="exercise",Text="Earlier stretch reminder.",MessageUtc=now.AddMinutes(-10),CheckpointUtc=now.AddMinutes(-10),Pending=true,Cycle=new ReminderCycleState{Category="exercise",StartedUtc=now.AddMinutes(-10),NextRepeatUtc=now.AddMinutes(10),RepeatsSent=1}};
   ReminderJournal.Write(Path.Combine(root,"reminder-state.json"),state,now);Call(form,"RestoreDesktopReminder",now);
   Check(!Get<bool>(form,"messagePending")&&!Get<ReminderCycle>(form,"cycle").Finished&&SameTime(Get<ReminderCycle>(form,"cycle").NextRepeatUtc,state.Cycle.NextRepeatUtc),"an expired bubble is not revived while a future repeat timer is retained");
   state.CheckpointUtc=now.AddMinutes(-8);state.SuspendedUtc=state.CheckpointUtc;ReminderJournal.Write(Path.Combine(root,"reminder-state.json"),state,now);Call(form,"RestoreDesktopReminder",now);
   Check(Get<ReminderCycle>(form,"cycle").Finished&&Read(root)==null,"restart after a recorded long suspension does not revive its future followups");
   state.SuspendedUtc=null;state.CheckpointUtc=now;state.Cycle.NextRepeatUtc=now.AddMinutes(-6);state.Pending=false;state.Cycle.HiddenByConfirmation=true;ReminderJournal.Write(Path.Combine(root,"reminder-state.json"),state,now);
   Call(form,"RestoreDesktopReminder",now);Check(Get<ReminderCycle>(form,"cycle").Finished&&Read(root)==null,"stale repeat deadlines are discarded on restoration");
  }finally{Close(form);}
  form=Subject();try{
   await (Task)Call(form,"StartDesktopReminder","exercise",true);var old=Get<PetMessage>(form,"petMessage");await (Task)Call(form,"StartDesktopReminder","exercise",true);old.Confirmed();
   Check(Get<bool>(form,"messagePending")&&Read(root).Pending,"a late confirmation from a replaced bubble cannot dismiss the new reminder");
   var settings=Get<Settings>(form,"settings");using(var activity=new NativeActivity(settings)){
    Check(activity.Available,"native hooks are available for the returned-input test");
    var now=DateTime.UtcNow;Set(form,"cycle",new ReminderCycle("exercise",now.AddMinutes(-10),settings));var filter=Get<ActivityFilter>(activity,"filter");Set(filter,"lastSignificant",now.AddMinutes(-6));filter.Keyboard(now);Set(form,"activity",activity);
    Call(form,"PersistDesktopReminder",DateTime.UtcNow);
    Check(Get<ReminderCycle>(form,"cycle").Finished&&Get<bool>(form,"messagePending")&&Read(root).Pending,"a completed idle gap stops followups without dismissing the unacknowledged message");
    var previous=Get<PetMessage>(form,"petMessage");string skipped="wake-"+Guid.NewGuid().ToString("N");
    await (Task<bool>)Call(form,"DeliverDueDesktopReminders",new List<RitualEvent>{new RitualEvent{Id=skipped,Category="exercise",Utc=now.AddMinutes(-2)}},false);
    Check(object.ReferenceEquals(previous,Get<PetMessage>(form,"petMessage"))&&settings.HandledEvents.Contains(skipped),"returning input does not replay a due event already covered by the completed rest");
    await (Task<bool>)Call(form,"DeliverDueDesktopReminders",new List<RitualEvent>{new RitualEvent{Id="after-"+skipped,Category="exercise",Utc=DateTime.UtcNow}},false);
    Check(!object.ReferenceEquals(previous,Get<PetMessage>(form,"petMessage"))&&!Get<ReminderCycle>(form,"cycle").Finished,"a later event starts its own cycle after the earlier rest");
    now=DateTime.UtcNow;Set(form,"cycle",new ReminderCycle("exercise",now.AddMinutes(-10),settings));Set(form,"reminderMessageUtc",now.AddMinutes(-10));int version=Get<int>(form,"audioVersion");
    Call(form,"HandleDesktopPower",PowerModes.Suspend,now.AddMinutes(-9));
    Check(Get<int>(form,"audioVersion")>version&&Get<bool>(form,"messagePending"),"suspend cancels old audio without acknowledging its message");
    Call(form,"HandleDesktopPower",PowerModes.Resume,now);Check(Get<ReminderCycle>(form,"cycle").Finished&&Get<bool>(form,"messagePending")&&activity.HasRestedSince(now.AddMinutes(-1)),"resume marks a long sleep as completed rest while the message still waits for acknowledgement");
    Set(form,"activity",null);
   }
  }finally{Close(form);}
  form=Subject();try{
   var settings=Get<Settings>(form,"settings");settings.ExerciseEnabled=true;settings.ExerciseUtc=DateTime.UtcNow.AddMinutes(30);
   await (Task)Call(form,"StartDesktopReminder","exercise",true);var message=Get<PetMessage>(form,"petMessage");
   Call(form,"SetDesktopExerciseEnabled",false);
   Check(!settings.ExerciseEnabled&&!settings.ExerciseUtc.HasValue&&!Get<bool>(form,"messagePending")&&Get<ReminderCycle>(form,"cycle")==null&&!message.Visible&&Read(root)==null,"disabling stretch clears its deadline, current bubble and persistent repeat chain");
   Call(form,"SetDesktopExerciseEnabled",true);
   Check(settings.ExerciseEnabled&&!settings.ExerciseUtc.HasValue&&Read(root)==null,"enabling stretch does not restore a cancelled breakfast timer");
   await (Task)Call(form,"StartDesktopReminder","lunch",true);message=Get<PetMessage>(form,"petMessage");Call(form,"SetDesktopExerciseEnabled",false);
   Check(object.ReferenceEquals(message,Get<PetMessage>(form,"petMessage"))&&message.Visible&&Read(root).Category=="lunch","disabling stretch preserves an unrelated meal reminder");
   settings.ExerciseEnabled=true;settings.ExerciseUtc=DateTime.UtcNow.AddMinutes(30);await (Task)Call(form,"StartDesktopReminder","exercise",true);
   DateTime? deadlineBefore=settings.ExerciseUtc;var cycleBefore=Get<ReminderCycle>(form,"cycle");
   using(var locked=new FileStream(Path.Combine(root,"settings.json"),FileMode.Open,FileAccess.Read,FileShare.Read))Call(form,"SetDesktopExerciseEnabled",false);
   Check(settings.ExerciseEnabled&&settings.ExerciseUtc==deadlineBefore&&object.ReferenceEquals(cycleBefore,Get<ReminderCycle>(form,"cycle"))&&Read(root).Pending,"a failed stretch preference save preserves the active reminder and timer");
   var draft=new System.Web.Script.Serialization.JavaScriptSerializer().Deserialize<Settings>(new System.Web.Script.Serialization.JavaScriptSerializer().Serialize(settings));draft.ExerciseEnabled=false;Call(form,"SaveDesktopSettings",draft);
   Check(!settings.ExerciseEnabled&&!settings.ExerciseUtc.HasValue&&!Get<bool>(form,"messagePending")&&Read(root)==null,"the simple settings save also cancels a disabled stretch chain");
  }finally{Close(form);}
  using(var control=new Control()){
   IntPtr handle=control.Handle;int thread=System.Threading.Thread.CurrentThread.ManagedThreadId,calls=0;
   using(var watcher=new DesktopPowerWatcher(control,delegate(PowerModes mode,DateTime observed){if(mode==PowerModes.Suspend){Check(System.Threading.Thread.CurrentThread.ManagedThreadId==thread&&observed<=DateTime.UtcNow,"power notification is marshalled onto the owning UI thread");calls++;}})){
    typeof(DesktopPowerWatcher).GetMethod("Changed",Private).Invoke(watcher,new object[]{null,new PowerModeChangedEventArgs(PowerModes.Suspend)});await Task.Delay(50);Check(calls==1,"power notification reaches the controller once");
    watcher.Dispose();typeof(DesktopPowerWatcher).GetMethod("Changed",Private).Invoke(watcher,new object[]{null,new PowerModeChangedEventArgs(PowerModes.Suspend)});await Task.Delay(50);Check(calls==1,"disposed power watcher cannot enqueue more callbacks");
   }
  }
 }
 [STAThread]public static int Main(string[] args){int at=Array.IndexOf(args,"--data-dir");if(at<0||!args.Contains("--self-test"))return 2;Application.EnableVisualStyles();Application.SetCompatibleTextRenderingDefault(false);int code=1;using(var host=new Form{Opacity=0,ShowInTaskbar=false,Width=1,Height=1}){host.Shown+=async delegate{try{await Run(Path.GetFullPath(args[at+1]));Console.WriteLine("PASS "+passed+" reminder restoration UI checks");code=0;}catch(Exception error){Console.Error.WriteLine(error);}finally{host.Close();}};Application.Run(host);}return code;}
}
