using System;
using System.IO;
using System.Collections.Generic;
using System.Linq;
using System.Threading.Tasks;
using Microsoft.Win32;

public partial class MainForm {
 string reminderCategory="",reminderText="";
 bool reminderFollowup;
 DateTime reminderMessageUtc=DateTime.MinValue,nextReminderCheckpointUtc=DateTime.MinValue;
 bool reminderJournalHasState,reminderJournalKnown;
 DateTime? desktopSuspendedUtc;
 void CancelDesktopExercise(){if(reminderCategory!="exercise"&&(cycle==null||cycle.Category!="exercise"))return;cycle=null;messagePending=false;manualMessage=false;StopAudio();if(petMessage!=null)petMessage.Hide();PersistDesktopReminder(DateTime.UtcNow);}
 void SetDesktopExerciseEnabled(bool enabled){
  bool previous=settings.ExerciseEnabled;DateTime? deadline=settings.ExerciseUtc;Schedule.SetExercise(settings,enabled);
  if(!Save()){settings.ExerciseEnabled=previous;settings.ExerciseUtc=deadline;}
  else if(!enabled)CancelDesktopExercise();
  refreshingAdvanced=true;try{exerciseToggle.Checked=settings.ExerciseEnabled;}finally{refreshingAdvanced=false;}UpdateLabels();
 }
 string ReminderStatePath {get{return Path.Combine(Folder,"reminder-state.json");}}
 bool RestCompletedForCycle(DateTime now){return cycle!=null&&activity!=null&&(activity.IsIdle(now)||activity.HasRestedSince(cycle.StartedUtc));}
 bool PersistDesktopReminder(DateTime now){
  nextReminderCheckpointUtc=now.AddSeconds(30);
  if(cycle!=null&&RestCompletedForCycle(now))cycle.Finished=true;
  ReminderCheckpoint saved=null;
  if((!settings.Paused||manualMessage)&&(messagePending||cycle!=null&&!cycle.Finished)&&ReminderCheckpoint.CategoryValid(reminderCategory)&&reminderMessageUtc!=DateTime.MinValue)
   saved=new ReminderCheckpoint{Category=reminderCategory,Text=reminderText,MessageUtc=reminderMessageUtc,CheckpointUtc=now,SuspendedUtc=desktopSuspendedUtc,Pending=messagePending,Manual=manualMessage,Followup=reminderFollowup,Cycle=ReminderCycleState.Capture(cycle)};
  if(saved==null&&!reminderJournalHasState&&(reminderJournalKnown||!File.Exists(ReminderStatePath)))return true;
  try{ReminderJournal.Write(ReminderStatePath,saved,now);reminderJournalKnown=true;reminderJournalHasState=saved!=null;return true;}
  catch(Exception error){LogDesktop(error);preparationStatus=L.T("Не удалось сохранить состояние напоминания; после перезапуска оно может повториться.","Could not save reminder state; it may repeat after a restart.");return false;}
 }
 void RestoreDesktopReminder(DateTime now){
  try{
   var saved=ReminderJournal.Read(ReminderStatePath,now);reminderJournalKnown=true;reminderJournalHasState=saved!=null;if(saved==null)return;
   if(settings.Paused&&!saved.Manual){ReminderJournal.Write(ReminderStatePath,null,now);reminderJournalHasState=false;return;}
   reminderCategory=saved.Category;reminderText=saved.Text;reminderFollowup=saved.Followup;reminderMessageUtc=saved.MessageUtc;manualMessage=saved.Manual;
   cycle=saved.Cycle==null?null:saved.Cycle.Restore(settings);
   if(cycle!=null&&saved.SuspendedUtc.HasValue&&cycle.StartedUtc<=saved.SuspendedUtc.Value&&now-saved.SuspendedUtc.Value>=TimeSpan.FromMinutes(settings.IdleMinutes))cycle.Finished=true;
   if(cycle!=null&&(RestCompletedForCycle(now)||!RitualSchedule.RepeatsAllowed(settings,cycle.Category,now)||now-cycle.NextRepeatUtc>TimeSpan.FromMinutes(5)))cycle.Finished=true;
   messagePending=saved.RestoreMessage(now);lastText.Text=saved.Text;
   if(messagePending)ShowDesktopMessage(saved.Text,saved.Category,saved.Followup);
   // Restored messages are silent and do not consume another library row.
   PersistDesktopReminder(now);
  }catch(Exception error){LogDesktop(error);preparationStatus=L.T("Не удалось восстановить прежнее напоминание. Новые напоминания работают.","Could not restore the previous reminder. New reminders still work.");}
 }
 void ShowDesktopMessage(string text,string category,bool followup){
  reminderText=text;
  if(petMessage!=null){petMessage.Close();petMessage=null;}
  var message=new PetMessage(text,category=="break"?L.T("Я встаю","I am getting up"):category=="exercise"?L.T("Иду разминаться","I am going to stretch"):L.T("Иду есть","I am going to eat"),L.T("Стоп голос","Stop voice"));petMessage=message;
  messagePending=true;
  message.Confirmed=delegate{if(!object.ReferenceEquals(petMessage,message))return;messagePending=false;StopAudio();if(cycle!=null)cycle.Confirm(DateTime.UtcNow,settings);PersistDesktopReminder(DateTime.UtcNow);};
  message.StopVoice=StopAudio;
  if(pet!=null){pet.Teasing=followup;pet.Happy=false;message.AnchorTo(pet);}
  if(!FullscreenDetector.IsFullscreen()||settings.FullscreenVisual!="none")message.Show();
 }
 async Task<bool> DeliverDueDesktopReminders(List<RitualEvent> due,bool idle){
  if(due.Count==0)return false;
  var eligible=due.Where(item=>!idle&&(activity==null||!activity.HasRestedSince(item.Utc))).ToArray();
  RitualSchedule.MarkHandled(settings,due);if(due.Any(item=>item.Category=="exercise"))settings.ExerciseUtc=null;Save();
  if(eligible.Length>0)await StartDesktopReminder(eligible[0].Category,true);
  return true;
 }
 void HandleDesktopPower(PowerModes mode,DateTime observed){
  if(mode==PowerModes.Suspend){
   desktopSuspendedUtc=observed;
   if(reminderMessageUtc<=observed)StopAudio();
   PersistDesktopReminder(DateTime.UtcNow);
  }else if(mode==PowerModes.Resume&&desktopSuspendedUtc.HasValue){
   DateTime suspended=desktopSuspendedUtc.Value;desktopSuspendedUtc=null;
   if(activity!=null)activity.ObserveSuspension(suspended,observed);
   if(observed-suspended>=TimeSpan.FromMinutes(settings.IdleMinutes)&&cycle!=null&&cycle.StartedUtc<observed)cycle.Finished=true;
   PersistDesktopReminder(DateTime.UtcNow);
  }
 }
}
