using System;
using System.Linq;
using System.Collections.Generic;

public partial class Settings {public bool ExerciseEnabled=true;public DateTime? ExerciseUtc;}

public static class RitualCoreTests {
 static int passed;
 static void Check(bool yes,string name){if(!yes)throw new Exception(name);Console.WriteLine("PASS "+name);passed++;}
 public static void Main(){
  var s=new Settings{TimeZoneId="UTC"};var monday=new DateTime(2026,10,19,10,0,0,DateTimeKind.Utc);
  var due=RitualSchedule.Due(s,monday);Check(due.Count==2&&due[0].Category=="breakfast","meal has priority over simultaneous break");
  Check(RitualSchedule.Next(s,monday).Category=="breakfast","overview shows priority at equal times");
  Check(RitualSchedule.Next(s,monday.AddDays(4).Date.AddHours(20)).Utc==monday.AddDays(7),"overview skips weekends");
  Check(RitualSchedule.Next(new Settings{TimeZoneId="UTC",WorkDays=new int[0]},monday)==null,"overview handles an empty schedule");
  RitualSchedule.MarkHandled(s,due);Check(RitualSchedule.Due(s,monday).Count==0,"event deduplication survives saved state");
  Check(RitualSchedule.Due(new Settings{TimeZoneId="UTC"},monday.AddMinutes(6)).Count==0,"no stale backlog");
  var saturday=monday.AddDays(5);s.ExerciseUtc=saturday;s.ExerciseEnabled=true;
  Check(RitualSchedule.Due(s,saturday).Single().Category=="exercise","breakfast exercise works on weekends");
  Check(RitualSchedule.Next(s,saturday).Category=="exercise","overview includes weekend exercise");
  s.WorkDays=new[]{0,6};Check(RitualSchedule.Day(s,saturday).Count>0&&RitualSchedule.Day(s,monday).Count==0,"custom work days");s.WorkDays=new[]{1,2,3,4,5};
  var a=new ActivityFilter(monday);for(int i=1;i<=3;i++)a.Click(monday.AddMinutes(i),s);
  Check(a.IsIdle(monday.AddMinutes(5),s),"three incidental clicks do not cancel break");
  a=new ActivityFilter(monday);for(int i=1;i<=4;i++)a.Click(monday.AddSeconds(i),s);
  Check(!a.IsIdle(monday.AddMinutes(5),s),"fourth click counts as activity");
  a=new ActivityFilter(monday);a.Keyboard(monday.AddMinutes(4));Check(!a.IsIdle(monday.AddMinutes(5),s),"keyboard counts as activity");
  a=new ActivityFilter(monday);a.Move(monday.AddSeconds(1),10);Check(a.IsIdle(monday.AddMinutes(5),s),"short cursor motion ignored");
  a=new ActivityFilter(monday);a.Move(monday.AddSeconds(1),90);Check(!a.IsIdle(monday.AddMinutes(5),s),"sustained movement counts");
  a.Scroll(monday.AddMinutes(4));Check(!a.IsIdle(monday.AddMinutes(6),s),"scroll counts");
  var c=new ReminderCycle("break",monday,s);c.Confirm(monday.AddMinutes(1),s);
  Check(!c.Finished&&!c.Tick(monday.AddMinutes(5),s,false)&&c.Tick(monday.AddMinutes(6),s,false),"untrusted confirmation hides then rechecks");
  Check(c.Tick(monday.AddMinutes(11),s,false)&&c.Tick(monday.AddMinutes(16),s,false)&&!c.Tick(monday.AddMinutes(21),s,false),"at most three followups");
  c=new ReminderCycle("break",monday,s);Check(!c.Tick(monday.AddMinutes(5),s,true)&&c.Finished,"inactivity finishes the cycle");
  c=new ReminderCycle("break",monday,s);s.TrustConfirmation=true;c.Confirm(monday,s);Check(c.Finished,"optional trust confirmation");s.TrustConfirmation=false;
  c=new ReminderCycle("lunch",monday,s);Check(!c.Tick(monday.AddMinutes(5),s,false),"meals have no default repeats");c.Confirm(monday,s);Check(c.Finished,"meal acknowledgement finishes");
  c=new ReminderCycle("break",monday.Date.AddHours(19),s);Check(!c.Tick(monday.Date.AddHours(19).AddMinutes(5),s,false),"no repeats outside working hours");
  var planning=new Settings{TimeZoneId="UTC",ExerciseEnabled=false};var plan=RitualSchedule.Preparation(planning,monday.AddHours(-1));
  Check(plan.Any(p=>p.Category=="breakfast"&&p.Utc==monday)&&!plan.Any(p=>p.Category=="break"&&p.Utc==monday),"preparation omits a break suppressed by a meal");
  Check(plan.Any(p=>p.Category=="break"&&p.Utc==monday.Date.AddHours(19))&&!plan.Any(p=>p.Followup&&p.Category=="break"&&p.Utc>monday.Date.AddHours(19)),"preparation omits repeats after working hours");
  Check(plan.Select(p=>p.Utc).SequenceEqual(plan.Select(p=>p.Utc).OrderBy(t=>t)),"preparation prioritizes the nearest actual playback");
  planning.BreakEvery=5;plan=RitualSchedule.Preparation(planning,monday);
  Check(!plan.Any(p=>p.Followup),"a new scheduled event replaces the previous chain instead of prefetching its repeats");
  planning=new Settings{TimeZoneId="UTC",ExerciseUtc=saturday};plan=RitualSchedule.Preparation(planning,saturday.AddMinutes(-1));
  Check(plan.Count==4&&plan.All(p=>p.Category=="exercise"),"weekend preparation contains exercise and its followups only");
  planning=new Settings{TimeZoneId="UTC",ExerciseUtc=monday};plan=RitualSchedule.Preparation(planning,monday);
  Check(!plan.Any(p=>p.Category=="exercise"),"meal priority also suppresses a coincident exercise in preparation");
  planning=new Settings{TimeZoneId="UTC"};plan=RitualSchedule.Preparation(planning,monday.AddHours(-1));
  Check(plan.Any(p=>p.Category=="break"&&p.Utc==monday.AddHours(1))&&plan.Count(p=>p.Category=="exercise")==4,"an unknown breakfast timer reserves exercise without suppressing real work events");
  s.BreakEvery=0;bool invalid=false;try{RitualSchedule.Day(s,monday);}catch(ArgumentException){invalid=true;}Check(invalid,"reject invalid interval without infinite loop");
  Console.WriteLine("PASS "+passed+" ritual checks");
 }
}
