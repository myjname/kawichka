using System;
using System.IO;
using System.Linq;
using System.Web.Script.Serialization;

public static class ReminderStateTests {
 static int passed;
 static void Check(bool yes,string name){if(!yes)throw new Exception(name);Console.WriteLine("PASS "+name);passed++;}
 public static int Main(){string work=Path.GetFullPath(Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"..","..","work"));string root=Path.Combine(work,"reminder-state-"+Guid.NewGuid().ToString("N"));try{
  Directory.CreateDirectory(root);var now=new DateTime(2026,10,26,10,0,0,DateTimeKind.Utc);var s=new Settings{TimeZoneId="UTC",WorkDays=new[]{0,1,2,3,4,5,6},WorkStart=0,WorkEnd=1439};
  var filter=new ActivityFilter(now,s);filter.Keyboard(now.AddMinutes(6));filter.Keyboard(now.AddMinutes(6).AddSeconds(1));
  Check(!filter.IsIdle(now.AddMinutes(6).AddSeconds(2),s)&&filter.CompletedRestUtc==now.AddMinutes(6),"returning keyboard input retains evidence of the completed rest");
  Check(filter.CompletedRestUtc>now&&filter.CompletedRestUtc<now.AddMinutes(7),"a completed rest resolves an earlier cycle but not a newly started one");
  var custom=new Settings{IdleMinutes=1};filter=new ActivityFilter(now,custom);filter.Scroll(now.AddSeconds(70));
  Check(filter.CompletedRestUtc==now.AddSeconds(70)&&!filter.IsIdle(now.AddSeconds(71),custom),"custom inactivity duration applies to a returning scroll");
  filter=new ActivityFilter(now,s);filter.Move(now.AddMinutes(6),100);Check(filter.CompletedRestUtc==now.AddMinutes(6),"returning sustained mouse movement also preserves the rest");
  filter=new ActivityFilter(now,s);for(int i=1;i<=3;i++)filter.Click(now.AddMinutes(i),s);filter.Keyboard(now.AddMinutes(6));
  Check(filter.CompletedRestUtc==now.AddMinutes(6),"allowed incidental clicks do not invalidate the completed rest");
  filter=new ActivityFilter(now,s);for(int i=1;i<=4;i++)filter.Click(now.AddMinutes(i),s);filter.Keyboard(now.AddMinutes(6));
  Check(filter.CompletedRestUtc==DateTime.MinValue,"a fourth click interrupts inactivity before a rest is completed");
  filter=new ActivityFilter(now,s);filter.ObserveSuspension(now,now.AddMinutes(4));Check(filter.CompletedRestUtc==DateTime.MinValue,"a short suspension does not count as a completed rest");
  filter.ObserveSuspension(now,now.AddMinutes(6));filter.Keyboard(now.AddMinutes(6).AddSeconds(1));Check(filter.CompletedRestUtc>=now.AddMinutes(6),"a long suspension remains a completed rest after returned input");
  var cycle=new ReminderCycle("break",now,s);Check(!cycle.Tick(now.AddMinutes(11),s,false)&&cycle.Finished&&cycle.RepeatsSent==0,"overdue followups beyond five minutes are discarded without catching up");
  cycle=new ReminderCycle("break",now,s);Check(cycle.Tick(now.AddMinutes(10),s,false)&&cycle.NextRepeatUtc==now.AddMinutes(15)&&!cycle.Tick(now.AddMinutes(11),s,false),"a followup at the grace boundary runs once and resets its interval from actual delivery");
  cycle=new ReminderCycle("break",now,s);cycle.Tick(now.AddMinutes(5),s,false);cycle.Tick(now.AddMinutes(10),s,false);
  string path=Path.Combine(root,"reminder-state.json");var saved=new ReminderCheckpoint{Category="break",Text="Take a short break.",MessageUtc=now.AddMinutes(10),CheckpointUtc=now.AddMinutes(10),Pending=true,Followup=true,Cycle=ReminderCycleState.Capture(cycle)};
  ReminderJournal.Write(path,saved,saved.CheckpointUtc);var read=ReminderJournal.Read(path,now.AddMinutes(12));var resumed=read.Cycle.Restore(s);
  Check(read.Pending&&read.Text==saved.Text&&resumed.RepeatsSent==2&&resumed.NextRepeatUtc==now.AddMinutes(15),"journal round trip preserves text, repeat count and the exact deadline");
  Check(!resumed.Tick(now.AddMinutes(14),s,false)&&resumed.Tick(now.AddMinutes(15),s,false)&&resumed.RepeatsSent==3&&resumed.Finished&&!resumed.Tick(now.AddMinutes(20),s,false),"the remaining repeat is delivered once and the exhausted cycle finishes");
  Check(read.RestoreMessage(now.AddMinutes(14))&&!read.RestoreMessage(now.AddMinutes(16)),"message restoration uses the five minute checkpoint grace");
  cycle=new ReminderCycle("exercise",now,s);cycle.Confirm(now.AddMinutes(1),s);
  saved=new ReminderCheckpoint{Category="exercise",Text="Stretch a little.",MessageUtc=now,CheckpointUtc=now.AddMinutes(1),Pending=false,Cycle=ReminderCycleState.Capture(cycle)};
  ReminderJournal.Write(path,saved,saved.CheckpointUtc);read=ReminderJournal.Read(path,now.AddMinutes(2));resumed=read.Cycle.Restore(s);
  Check(!read.RestoreMessage(now.AddMinutes(2))&&resumed.HiddenByConfirmation&&!resumed.Finished&&resumed.NextRepeatUtc==now.AddMinutes(6),"untrusted confirmation stays hidden after restart and retains its verification timer");
  string previous=File.ReadAllText(path);bool refused=false;
  using(var held=new FileStream(path,FileMode.Open,FileAccess.Read,FileShare.Read)){try{ReminderJournal.Write(path,null,now.AddMinutes(2));}catch(IOException){refused=true;}}
  Check(refused&&File.ReadAllText(path)==previous&&!Directory.GetFiles(root,"*.tmp").Any(),"a failed journal replacement preserves its previous state and cleans temporary files");
  saved.Version=2;refused=false;try{ReminderJournal.Write(path,saved,now.AddMinutes(2));}catch(InvalidDataException){refused=true;}
  Check(refused&&File.ReadAllText(path)==previous,"unsupported journal versions never overwrite existing state");
  File.WriteAllText(path,new string('x',32769));refused=false;try{ReminderJournal.Read(path,now);}catch(InvalidDataException){refused=true;}Check(refused,"oversized state is rejected before deserialization");
  File.WriteAllText(path,"{\"Version\":1,\"Category\":\"break\",\"Text\":\"old\"}");refused=false;try{ReminderJournal.Read(path,now);}catch(InvalidDataException){refused=true;}Check(refused,"incomplete state is rejected instead of creating a fresh cycle");
  ReminderJournal.Write(path,null,now);Check(ReminderJournal.Read(path,now)==null,"clearing state is durable");
  Console.WriteLine("PASS "+passed+" reminder state checks");return 0;
 }catch(Exception error){Console.Error.WriteLine(error);return 1;}finally{if(Path.GetFullPath(root).StartsWith(work+Path.DirectorySeparatorChar,StringComparison.OrdinalIgnoreCase)&&Path.GetFileName(root).StartsWith("reminder-state-",StringComparison.Ordinal)&&Directory.Exists(root))Directory.Delete(root,true);}}
}
