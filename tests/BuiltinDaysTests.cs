using System;
using System.IO;
using System.Linq;
using System.Collections.Generic;
using System.Threading.Tasks;

public static class BuiltinDaysTests {
 static int passed;
 static void Check(bool yes,string name){if(!yes)throw new Exception(name);Console.WriteLine("PASS "+name);passed++;}
 public static int Main(){
  string work=Path.GetFullPath(Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"..","..","work")),root=Path.Combine(work,"builtin-tests-"+Guid.NewGuid().ToString("N"));bool success=false;
  try{
   var monday=new DateTime(2026,10,19,9,0,0,DateTimeKind.Utc);var s=new Settings{TimeZoneId="UTC",SpeechLanguage="ru"};var store=new BuiltinDays(root);var day=store.Prepare(s,monday);
   Check(day.Rows.Count==RitualSchedule.PlannedCount(s,monday)&&day.Rows.Count>20,"basic mode prepares the complete schedule and followups");
   Check(day.Rows.Select(row=>row.Text).Distinct().Count()==day.Rows.Count,"the prepared day contains no identical phrases");
   Check(day.Rows.All(row=>row.Text.Length>=4&&row.Text.Length<=400),"all stock phrases fit speech and text limits");
   var ids=day.Rows.Select(row=>row.Id).ToArray();Check(ids.SequenceEqual(store.Prepare(s,monday.AddHours(2)).Rows.Select(row=>row.Id)),"repeated preparation preserves the original list");
   var consumed=store.Consume(s,monday,"break",false,false);Check(consumed!=null&&!string.IsNullOrEmpty(consumed.UsedAt),"consumption persists a reservation");
   store=new BuiltinDays(root);var second=store.Consume(s,monday,"break",false,false);Check(second!=null&&consumed.Id!=second.Id&&consumed.Text!=second.Text,"restarting does not repeat a consumed phrase");
   var teaser=store.Consume(s,monday,"break",true,false);Check(teaser!=null&&teaser.Followup&&teaser.Text!=second.Text,"teasing followups have their own queue");
   var next=store.Prepare(s,monday.AddDays(1));Check(next.Day!=day.Day&&next.Rows.All(item=>!day.Rows.Any(old=>old.Text==item.Text)),"the next day uses fresh variants while the bank has unused wording");
   Check(store.Read(s,monday).Rows.Any(item=>item.Id==consumed.Id&&item.UsedAt==consumed.UsedAt),"old text and playback history remain after day rollover");
   s.SpeechLanguage="en";var english=store.Prepare(s,monday);Check(english.Language=="en"&&english.Rows.All(item=>!item.Text.Any(c=>c>='А'&&c<='я')),"English has an independent prepared list");
   Check(store.Consume(s,monday,"break",false,false).Id!=consumed.Id,"language switching does not reuse another language's reservation");
   s.SpeechLanguage="ru";Check(store.Read(s,monday).Rows.Any(item=>item.Id==consumed.Id&&item.UsedAt==consumed.UsedAt),"returning to Russian retains its playback state");
   string reviewRoot=Path.Combine(root,"review");var review=new BuiltinDays(reviewRoot);var candidates=review.Prepare(s,monday,false);Check(candidates.Rows.All(item=>!item.Approved)&&review.Consume(s,monday,"break",false,false)==null,"manual review never consumes an unapproved candidate");
   var kept=candidates.Rows.First(item=>item.Category=="break"&&!item.Followup);review.Select(s,monday,new[]{kept.Id},"keep");Check(new BuiltinDays(reviewRoot).Consume(s,monday,"break",false,false).Id==kept.Id,"manual keep survives a new store instance");
   var rejected=candidates.Rows.First(item=>item.Followup);review.Select(s,monday,new[]{rejected.Id},"exclude");var replaced=review.Prepare(s,monday,false);Check(replaced.Rows.Count==candidates.Rows.Count+1&&replaced.Rows.Any(item=>item.Id==rejected.Id&&item.Rejected)&&replaced.Rows.Any(item=>item.Slot==rejected.Slot&&item.Id!=rejected.Id&&!item.Approved),"exclusion retains history and creates one reviewable replacement");
   Check(review.Prepare(s,monday,false).Rows.Count==replaced.Rows.Count,"waiting review does not create endless replacement candidates");
   review.Select(s,monday,new[]{kept.Id},"delete");Check(review.Read(s,monday).Rows.Any(item=>item.Id==kept.Id&&item.Deleted&&item.UsedAt.Length>0),"deletion retains text and played history");
   var manual=review.Add(s,monday,"exercise",7);Check(manual.Rows.Count(item=>item.Kind=="manual")==7&&manual.Rows.Where(item=>item.Kind=="manual").All(item=>!item.Approved),"manual batch count creates candidates without using a model");
   var fallback=review.Consume(s,monday,"exercise",false);Check(fallback.Kind=="fallback"&&fallback.Approved&&fallback.UsedAt.Length>0&&!candidates.Rows.Any(item=>item.Text==fallback.Text),"a missing approved entry gets a distinct emergency phrase");
   string export=review.Export(s,monday,new[]{fallback.Id});Check(export.Contains(fallback.Text)&&!export.Contains(kept.Text),"export respects selected phrases");
   var weekend=new BuiltinDays(Path.Combine(root,"weekend")).Prepare(s,monday.AddDays(5));Check(weekend.Rows.Count==4&&weekend.Rows.All(item=>item.Category=="exercise"),"weekend list only reserves independent exercise reminders");
   var concurrent=new BuiltinDays(Path.Combine(root,"concurrent"));concurrent.Prepare(s,monday);var claims=new System.Collections.Concurrent.ConcurrentBag<string>();Parallel.For(0,30,i=>{var item=concurrent.Consume(s,monday,"break",false,false);if(item!=null)claims.Add(item.Id);});Check(claims.Count>0&&claims.Count==claims.Distinct().Count(),"concurrent consumption reserves each phrase once");
   string lockedFile=Path.Combine(root,"builtin-days","2026-10-19.ru.json");var before=store.Read(s,monday).Rows.Count(item=>item.UsedAt.Length>0);bool failed=false;using(var locked=new FileStream(lockedFile,FileMode.Open,FileAccess.Read,FileShare.Read)){try{store.Consume(s,monday,"break",false,false);}catch(IOException){failed=true;}}
   Check(failed&&store.Read(s,monday).Rows.Count(item=>item.UsedAt.Length>0)==before,"a failed reservation write leaves the phrase available");
   string corruptRoot=Path.Combine(root,"corrupt"),corruptFile=Path.Combine(corruptRoot,"builtin-days","2026-10-19.ru.json");Directory.CreateDirectory(Path.GetDirectoryName(corruptFile));File.WriteAllText(corruptFile,"{broken");failed=false;try{new BuiltinDays(corruptRoot).Prepare(s,monday);}catch(Exception){failed=true;}Check(failed&&File.ReadAllText(corruptFile)=="{broken","corrupt current history is not overwritten with a fresh list");
   Check(!File.Exists(Path.Combine(root,"library.sqlite3"))&&!Directory.Exists(Path.Combine(root,"models"))&&!Directory.Exists(Path.Combine(root,"runtime")),"daily list needs no SQLite, Python or model files");
   Console.WriteLine("PASS "+passed+" built-in day checks");success=true;return 0;
  }catch(Exception error){Console.Error.WriteLine(error);Console.Error.WriteLine("Fixtures: "+root);return 1;}
  finally{if(success&&root.StartsWith(work+Path.DirectorySeparatorChar,StringComparison.OrdinalIgnoreCase)&&Path.GetFileName(root).StartsWith("builtin-tests-",StringComparison.Ordinal)&&Directory.Exists(root))Directory.Delete(root,true);}
 }
}
