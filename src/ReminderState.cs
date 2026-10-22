using System;
using System.IO;
using System.Linq;
using System.Web.Script.Serialization;

public sealed class ReminderCycleState {
 public string Category="";
 public DateTime StartedUtc,NextRepeatUtc;
 public int RepeatsSent;
 public bool Finished,HiddenByConfirmation;
 public static ReminderCycleState Capture(ReminderCycle cycle){return cycle==null?null:new ReminderCycleState{Category=cycle.Category,StartedUtc=cycle.StartedUtc,NextRepeatUtc=cycle.NextRepeatUtc,RepeatsSent=cycle.RepeatsSent,Finished=cycle.Finished,HiddenByConfirmation=cycle.HiddenByConfirmation};}
 public ReminderCycle Restore(Settings settings){return new ReminderCycle(Category,StartedUtc,settings){NextRepeatUtc=NextRepeatUtc,RepeatsSent=RepeatsSent,Finished=Finished||RepeatsSent>=RitualSchedule.Repeats(settings,Category),HiddenByConfirmation=HiddenByConfirmation};}
}

public sealed class ReminderCheckpoint {
 public int Version=1;
 public string Category="",Text="";
 public DateTime MessageUtc,CheckpointUtc;
 public DateTime? SuspendedUtc;
 public bool Pending,Manual,Followup;
 public ReminderCycleState Cycle;
 public static bool CategoryValid(string category){return new[]{"break","breakfast","lunch","dinner","exercise"}.Contains(category);}
 public void Validate(DateTime now){
  if(Version!=1)throw new InvalidDataException(AppText.T("Неподдерживаемая версия состояния напоминания","Unsupported reminder state"));
  if(!CategoryValid(Category)||Text==null||Text.Length>400||CheckpointUtc==DateTime.MinValue||CheckpointUtc>now.AddMinutes(5)||MessageUtc==DateTime.MinValue||MessageUtc>CheckpointUtc||Pending&&Text.Length==0||Manual&&Cycle!=null)throw new InvalidDataException(AppText.T("Неверное состояние напоминания","Invalid reminder state"));
  if(SuspendedUtc.HasValue&&(SuspendedUtc.Value==DateTime.MinValue||SuspendedUtc.Value>CheckpointUtc))throw new InvalidDataException(AppText.T("Неверная отметка сна в состоянии напоминания","Invalid suspended reminder state"));
  if(Cycle!=null&&(!CategoryValid(Cycle.Category)||Cycle.Category!=Category||Cycle.StartedUtc==DateTime.MinValue||Cycle.StartedUtc>CheckpointUtc||Cycle.NextRepeatUtc<Cycle.StartedUtc||Cycle.NextRepeatUtc>CheckpointUtc.AddHours(2)||Cycle.RepeatsSent<0||Cycle.RepeatsSent>20||Cycle.HiddenByConfirmation&&Pending))throw new InvalidDataException(AppText.T("Неверная серия напоминаний","Invalid reminder cycle"));
 }
 public bool RestoreMessage(DateTime now){return Pending&&now-CheckpointUtc<=TimeSpan.FromMinutes(5);}
}

public static class ReminderJournal {
 public static ReminderCheckpoint Read(string path,DateTime now){
  if(!File.Exists(path))return null;
  if((File.GetAttributes(path)&FileAttributes.ReparsePoint)!=0||new FileInfo(path).Length>32768)throw new InvalidDataException(AppText.T("Неверный файл состояния напоминания","Invalid reminder state file"));
  var saved=new JavaScriptSerializer().Deserialize<ReminderCheckpoint>(File.ReadAllText(path));
  if(saved!=null)saved.Validate(now);return saved;
 }
 public static void Write(string path,ReminderCheckpoint state,DateTime now){
  if(state!=null)state.Validate(now);
  SettingsFile.Write(path,new JavaScriptSerializer().Serialize(state));
 }
}
