using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;

public partial class Settings {
 public int SchemaVersion=2;
 public bool SetupComplete=false;
 public bool PersonalizationMigrated=false;
 public bool BackendPersonalizationMigrated=false;
 public bool BackendSettingsMigrated=false;
 public bool BackendDailySettingsMigrated=false;
 public List<string> BackendSettingsOverrides=new List<string>();
 public string LastNotifiedUpdate="";
 public string UiLanguage=CultureInfo.CurrentUICulture.TwoLetterISOLanguageName=="ru"?"ru":"en";
 public string SpeechLanguage=CultureInfo.CurrentUICulture.TwoLetterISOLanguageName=="ru"?"ru":"en";
 public string ThemeMode="system";
 public string AddressForm="neutral";
 public string UserName="";
 public string TimeZoneId="";
 public int[] WorkDays=new[]{1,2,3,4,5};
 public int WorkStart=600,WorkEnd=1140,BreakEvery=60;
 public int BreakfastTime=600,LunchTime=870,DinnerTime=1110;
 public bool MealsEnabled=true;
 public int ExerciseAfter=120;
 public int IdleMinutes=5,AllowedClicks=3,RepeatMinutes=5,BreakRepeats=3,ExerciseRepeats=3,MealRepeats=0;
 public bool TrustConfirmation=false;
 public DateTime? PauseUntilUtc;
 public string FullscreenVisual="both",FullscreenAudio="all";
 public bool DeferGenerationFullscreen=true;
 public int PetX=int.MinValue,PetY=int.MinValue,PetSize=130;
 public string VisualFile="",VoiceProfile="",CharacterFile="";
 public bool PetVisible=true,AutomaticUpdates=true;
 public bool RunAtLogin=false;
 public bool AutomaticPreparation=true,AutoKeepDaily=true;
 public bool TextGeneration=false;
 public bool RuntimeGeneration=false;
 public string TextModel="text-gemma4-12b",RuntimeTextModel="text-gemma4-12b",VoiceModel="tts-cosyvoice3";
 public List<string> HandledEvents=new List<string>();
}

public sealed class RitualEvent {
 public string Id,Category;
 public DateTime Utc;
 public int Priority {get{return Category=="break"?0:Category=="exercise"?1:2;}}
}

public sealed class PreparationSlot {
 public string Id,Category;
 public DateTime Utc;
 public bool Followup;
}

public static class RitualSchedule {
 public static TimeZoneInfo Zone(Settings s) {try{return string.IsNullOrEmpty(s.TimeZoneId)?TimeZoneInfo.Local:TimeZoneInfo.FindSystemTimeZoneById(s.TimeZoneId);}catch(TimeZoneNotFoundException){return TimeZoneInfo.Local;}}
 public static DateTime Local(Settings s,DateTime utc){return TimeZoneInfo.ConvertTimeFromUtc(DateTime.SpecifyKind(utc,DateTimeKind.Utc),Zone(s));}
 public static DateTime Utc(Settings s,DateTime local) {
  local=DateTime.SpecifyKind(local,DateTimeKind.Unspecified);var zone=Zone(s);
  while(zone.IsInvalidTime(local))local=local.AddMinutes(1);
  return TimeZoneInfo.ConvertTimeToUtc(local,zone);
 }
 public static void Validate(Settings s) {
  if(s.WorkDays==null||s.WorkDays.Any(d=>d<0||d>6)||s.WorkDays.Distinct().Count()!=s.WorkDays.Length)throw new ArgumentException(AppText.T("Неверно выбраны рабочие дни","Invalid work days"));
  if(s.WorkStart<0||s.WorkEnd>=1440||s.WorkStart>s.WorkEnd||s.BreakEvery<5||s.BreakEvery>720)throw new ArgumentException(AppText.T("Неверно задан рабочий период","Invalid work period"));
  if(new[]{s.BreakfastTime,s.LunchTime,s.DinnerTime}.Any(t=>t<0||t>=1440))throw new ArgumentException(AppText.T("Неверно задано время еды","Invalid meal time"));
  if(s.IdleMinutes<1||s.IdleMinutes>120||s.AllowedClicks<0||s.AllowedClicks>100||s.RepeatMinutes<1||s.RepeatMinutes>120||new[]{s.BreakRepeats,s.ExerciseRepeats,s.MealRepeats}.Any(n=>n<0||n>20)||s.ExerciseAfter<1||s.ExerciseAfter>1440)throw new ArgumentException(AppText.T("Неверные ограничения напоминаний","Invalid reminder limits"));
 }
 public static List<RitualEvent> Day(Settings s,DateTime localDay) {
  Validate(s);var result=new List<RitualEvent>();
  if(s.WorkDays.Contains((int)localDay.DayOfWeek)) {
   for(int minute=s.WorkStart;minute<=s.WorkEnd;minute+=s.BreakEvery)result.Add(Make(s,localDay,minute,"break"));
   if(s.MealsEnabled){result.Add(Make(s,localDay,s.BreakfastTime,"breakfast"));result.Add(Make(s,localDay,s.LunchTime,"lunch"));result.Add(Make(s,localDay,s.DinnerTime,"dinner"));}
  }
  return result.OrderBy(e=>e.Utc).ThenByDescending(e=>e.Priority).ToList();
 }
 static RitualEvent Make(Settings s,DateTime day,int minute,string category){return new RitualEvent{Category=category,Utc=Utc(s,day.Date.AddMinutes(minute)),Id=day.ToString("yyyy-MM-dd")+":"+minute+":"+category};}
 public static List<RitualEvent> Due(Settings s,DateTime utc) {
  if(s.HandledEvents==null)s.HandledEvents=new List<string>();
  var events=Day(s,Local(s,utc).Date);
  if(s.ExerciseEnabled&&s.ExerciseUtc.HasValue)events.Add(new RitualEvent{Id="exercise:"+s.ExerciseUtc.Value.ToString("o"),Category="exercise",Utc=s.ExerciseUtc.Value});
  return events.Where(e=>e.Utc<=utc&&utc-e.Utc<=TimeSpan.FromMinutes(5)&&!s.HandledEvents.Contains(e.Id)).OrderByDescending(e=>e.Priority).ThenByDescending(e=>e.Utc).ToList();
 }
 public static bool WithinWorkPeriod(Settings s,DateTime utc){var d=Local(s,utc);return s.WorkDays.Contains((int)d.DayOfWeek)&&d.TimeOfDay.TotalMinutes>=s.WorkStart&&d.TimeOfDay.TotalMinutes<s.WorkEnd+1;}
 public static RitualEvent Next(Settings s,DateTime utc){
  var events=new List<RitualEvent>();var day=Local(s,utc).Date;
  for(int i=0;i<8;i++)events.AddRange(Day(s,day.AddDays(i)));
  if(s.ExerciseEnabled&&s.ExerciseUtc.HasValue)events.Add(new RitualEvent{Id="exercise:"+s.ExerciseUtc.Value.ToString("o"),Category="exercise",Utc=s.ExerciseUtc.Value});
  return events.Where(e=>e.Utc>=utc&&(s.HandledEvents==null||!s.HandledEvents.Contains(e.Id))).OrderBy(e=>e.Utc).ThenByDescending(e=>e.Priority).FirstOrDefault();
 }
 public static bool RepeatsAllowed(Settings s,string category,DateTime utc){return category=="exercise"||WithinWorkPeriod(s,utc);}
 public static int Repeats(Settings s,string category){return category=="break"?s.BreakRepeats:category=="exercise"?s.ExerciseRepeats:s.MealRepeats;}
 public static List<PreparationSlot> Preparation(Settings s,DateTime utc){
  var day=Local(s,utc).Date;var events=Day(s,day);
  if(s.ExerciseEnabled&&s.ExerciseUtc.HasValue){var time=s.ExerciseUtc.Value;if(Local(s,time).Date==day&&time>=utc.AddMinutes(-5))events.Add(new RitualEvent{Id="exercise",Category="exercise",Utc=time});}
  // The dispatcher starts one chain for coincident events. A subsequent event
  // replaces that chain, and work reminders never repeat after working hours.
  events=events.GroupBy(e=>e.Utc).Select(group=>group.OrderByDescending(e=>e.Priority).First()).OrderBy(e=>e.Utc).ToList();
  var result=new List<PreparationSlot>();
  for(int i=0;i<events.Count;i++){
   var e=events[i];if(e.Utc<utc.AddMinutes(-5))continue;
   for(int repeat=0;repeat<=Repeats(s,e.Category);repeat++){
    var time=e.Utc.AddMinutes(repeat*s.RepeatMinutes);
    if(repeat>0&&(!RepeatsAllowed(s,e.Category,time)||(i+1<events.Count&&time>=events[i+1].Utc)))break;
    if(Local(s,time).Date!=day)break;
    result.Add(new PreparationSlot{Id=e.Id+":"+repeat,Category=e.Category,Utc=time,Followup=repeat>0});
   }
  }
  if(s.ExerciseEnabled&&!s.ExerciseUtc.HasValue){var reserve=utc.AddMinutes(s.ExerciseAfter);for(int n=0;n<=s.ExerciseRepeats;n++){var time=reserve.AddMinutes(n*s.RepeatMinutes);if(Local(s,time).Date!=day)break;result.Add(new PreparationSlot{Id="exercise:"+n,Category="exercise",Utc=time,Followup=n>0});}}
  return result.OrderBy(slot=>slot.Utc).ToList();
 }
 public static int PlannedCount(Settings s,DateTime day){return Preparation(s,Utc(s,day.Date)).Count;}
 public static void MarkHandled(Settings s,IEnumerable<RitualEvent> events){foreach(var e in events)if(!s.HandledEvents.Contains(e.Id))s.HandledEvents.Add(e.Id);if(s.HandledEvents.Count>600)s.HandledEvents.RemoveRange(0,s.HandledEvents.Count-600);}
}

// Only times, counts and mouse distance are retained; never keys or typed text.
public sealed class ActivityFilter {
 readonly Queue<DateTime> clicks=new Queue<DateTime>();
 readonly Settings settings;
 DateTime lastSignificant,motionStarted;
 DateTime completedRestUtc=DateTime.MinValue;
 double distance;
 public ActivityFilter(DateTime lastActivity,Settings liveSettings=null){settings=liveSettings??new Settings();lastSignificant=lastActivity;motionStarted=lastActivity;}
 void ObserveRest(DateTime now,Settings s){if(now-lastSignificant>=TimeSpan.FromMinutes(s.IdleMinutes))completedRestUtc=now;}
 void Significant(DateTime now,Settings s){ObserveRest(now,s);lastSignificant=now;}
 public void Keyboard(DateTime now){Significant(now,settings);}
 public void Scroll(DateTime now){Significant(now,settings);}
 public void Click(DateTime now,Settings s) {
  while(clicks.Count>0&&now-clicks.Peek()>=TimeSpan.FromMinutes(s.IdleMinutes))clicks.Dequeue();
  clicks.Enqueue(now);if(clicks.Count>s.AllowedClicks)Significant(now,s);
 }
 public void Move(DateTime now,double pixels) {
  if(now-motionStarted>TimeSpan.FromSeconds(3)){motionStarted=now;distance=0;}
  distance+=Math.Max(0,pixels);if(distance>=80){Significant(now,settings);motionStarted=now;distance=0;}
 }
 public bool IsIdle(DateTime now,Settings s){bool idle=now-lastSignificant>=TimeSpan.FromMinutes(s.IdleMinutes);if(idle)ObserveRest(now,s);return idle;}
 public DateTime LastSignificant {get{return lastSignificant;}}
 public DateTime CompletedRestUtc {get{return completedRestUtc;}}
 public void ObserveSuspension(DateTime startedUtc,DateTime resumedUtc){if(resumedUtc-startedUtc>=TimeSpan.FromMinutes(settings.IdleMinutes)&&resumedUtc>completedRestUtc)completedRestUtc=resumedUtc;}
}

public sealed class ReminderCycle {
 public readonly string Category;
 public readonly DateTime StartedUtc;
 public int RepeatsSent;
 public bool Finished,HiddenByConfirmation;
 public DateTime NextRepeatUtc;
 public ReminderCycle(string category,DateTime now,Settings s){Category=category;StartedUtc=now;NextRepeatUtc=now.AddMinutes(s.RepeatMinutes);Finished=RitualSchedule.Repeats(s,category)==0;}
 public void Confirm(DateTime now,Settings s){HiddenByConfirmation=true;if(s.TrustConfirmation||Category!="break"&&Category!="exercise")Finished=true;else NextRepeatUtc=now.AddMinutes(s.RepeatMinutes);}
 public bool Tick(DateTime now,Settings s,bool idle) {
  if(Finished)return false;
  if(idle){Finished=true;return false;}
  if(!RitualSchedule.RepeatsAllowed(s,Category,now)){Finished=true;return false;}
  if(RepeatsSent>=RitualSchedule.Repeats(s,Category)||now-NextRepeatUtc>TimeSpan.FromMinutes(5)){Finished=true;return false;}
  if(now<NextRepeatUtc)return false;
  RepeatsSent++;HiddenByConfirmation=false;NextRepeatUtc=now.AddMinutes(s.RepeatMinutes);if(RepeatsSent>=RitualSchedule.Repeats(s,Category))Finished=true;return true;
 }
}
