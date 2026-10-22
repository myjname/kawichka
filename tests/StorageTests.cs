using System;
using System.IO;
using System.Linq;
using System.Threading;
using System.Web.Script.Serialization;

public static class StorageTests {
 static int passed;
 static void Check(bool ok,string name){if(!ok)throw new Exception(name);Console.WriteLine("PASS "+name);passed++;}
 public static int Main(string[] args) {
  try {
   string root=Path.GetFullPath(args[0]),local=Path.Combine(root,"profile"),app=Path.Combine(root,"old version"),next=Path.Combine(root,"new version");Directory.CreateDirectory(root);
   string shared=SharedStorage.Resolve(app,local,new string[0]);
   Check(shared==SharedStorage.Resolve(next,local,new string[0]),"different application folders use the same shared storage");
   Check(shared==Path.Combine(local,"Kavychka"),"shared storage belongs to the user profile");
   foreach(string language in new[]{"en","ru"}){AppText.Language=language;Check(SharedStorage.Resolve(app,local,new string[0])==shared,"storage identity is independent of UI language "+language);}
   string isolated=Path.Combine(root,"isolated");
   Check(SharedStorage.Resolve(app,local,new[]{"--data-dir",isolated})==isolated,"explicit test data stays isolated");
   Check(SharedStorage.IsIsolated(new[]{"--preview"})&&SharedStorage.IsIsolated(new[]{"--data-dir",isolated}),"preview and test runs never import real user data");
   string old=Path.Combine(app,"data");Directory.CreateDirectory(Path.Combine(old,"models","tts-0.6b"));Directory.CreateDirectory(Path.Combine(old,"audio"));Directory.CreateDirectory(Path.Combine(old,"runtime"));
   File.WriteAllText(Path.Combine(old,"models","tts-0.6b","test.bin"),"legacy-model");File.WriteAllText(Path.Combine(old,"audio","test.flac"),"audio");
   File.WriteAllText(Path.Combine(old,"runtime","python.exe"),"python-test-fixture");File.WriteAllText(Path.Combine(old,"runtime-ok.json"),"{}");
   Directory.CreateDirectory(Path.Combine(old,"runtime","__pycache__"));File.WriteAllText(Path.Combine(old,"runtime","__pycache__","cached.pyc"),"cache");
   Directory.CreateDirectory(Path.Combine(old,"runtime","~pip-backup"));File.WriteAllText(Path.Combine(old,"runtime","~pip-backup","unused.bin"),"backup");
   File.WriteAllText(Path.Combine(old,"settings.json"),"{\"Voice\":false,\"LastBreak\":\"remember-me\",\"Secret\":\"test-only-secret\",\"ElevenSecret\":\"test-only-secret\"}");
   var pending=SharedStorage.Pending(app,local,shared);Check(pending.Length==1&&pending[0]==old,"legacy sibling data detected");
   SharedStorage.Preferences(shared,pending);string prefs=File.ReadAllText(Path.Combine(shared,"settings.json"));
   Check(prefs.Contains("remember-me")&&prefs.Contains("false"),"schedule and user choices preserved");Check(!prefs.Contains("Secret")&&!prefs.Contains("test-only-secret"),"legacy API secrets discarded");
   string redirected=Path.Combine(local,"Packages","OpenAI.Codex_2p2nqsd0c76g0","LocalCache","Local","Kavychka");
   Directory.CreateDirectory(Path.Combine(redirected,"models","text-gemma4-12b"));
   File.WriteAllText(Path.Combine(redirected,"models","text-gemma4-12b","model.gguf"),"model");
   File.WriteAllText(Path.Combine(redirected,"models","text-gemma4-12b","model.gguf.part"),"incomplete");
   foreach(string component in new[]{"llama","cosy"}) {Directory.CreateDirectory(Path.Combine(redirected,component));File.WriteAllText(Path.Combine(redirected,component,"verified.json"),"{}");}
   Check(SharedStorage.Pending(app,local,shared).Contains(redirected),"packaged Codex storage discovered");
   SharedStorage.Stage(shared,new[]{redirected},CancellationToken.None,(m,d,t)=>{});
   Check(File.Exists(Path.Combine(shared,"models","text-gemma4-12b","model.gguf"))&&!File.Exists(Path.Combine(shared,"models","text-gemma4-12b","model.gguf.part")),"redirected model imported without partial download");
   Check(File.Exists(Path.Combine(shared,"llama","verified.json"))&&File.Exists(Path.Combine(shared,"cosy","verified.json")),"both inference runtimes imported");
   SharedStorage.Complete(shared,new[]{redirected});
   int reports=0;SharedStorage.Stage(shared,pending,CancellationToken.None,(m,d,t)=>reports++);
   Check(File.ReadAllText(Path.Combine(shared,"models","tts-0.6b","test.bin"))=="legacy-model","model reused without a network download");
   Check(File.Exists(Path.Combine(shared,"runtime","python.exe"))&&File.Exists(Path.Combine(shared,"runtime-ok.json")),"runtime and installation marker reused");
   Check(!Directory.Exists(Path.Combine(shared,"runtime","__pycache__"))&&!Directory.Exists(Path.Combine(shared,"runtime","~pip-backup")),"bytecode caches and pip leftovers are not migrated");
   Check(File.Exists(Path.Combine(shared,"audio","test.flac"))&&reports>0,"audio copied with progress");
   Check(File.Exists(Path.Combine(old,"audio","test.flac")),"legacy source retained as a backup");
   File.WriteAllText(Path.Combine(shared,"models","tts-0.6b","test.bin"),"current-model");SharedStorage.Stage(shared,pending,CancellationToken.None,(m,d,t)=>{});
   Check(File.ReadAllText(Path.Combine(shared,"models","tts-0.6b","test.bin"))=="current-model","existing shared model never overwritten by a stale copy");
   var cancel=new CancellationTokenSource();cancel.Cancel();bool cancelled=false;
   try{SharedStorage.Stage(Path.Combine(root,"cancelled"),pending,cancel.Token,(m,d,t)=>{});}catch(OperationCanceledException){cancelled=true;}
   Check(cancelled&&File.Exists(Path.Combine(old,"runtime","python.exe")),"interrupted migration preserves originals");
   SharedStorage.Complete(shared,pending);Check(SharedStorage.Pending(app,local,shared).Length==0,"completed sources are not imported again");
   Check(SharedStorage.Pending(next,local,shared).Length==0&&File.Exists(Path.Combine(shared,"audio","test.flac")),"another release immediately sees shared data");
   string moved=Path.Combine(root,"different-disk-storage");bool verified=false;
   int remaining=SharedStorage.Relocate(shared,moved,local,CancellationToken.None,(m,d,t)=>{},delegate(string target){verified=File.ReadAllText(Path.Combine(target,"models","tts-0.6b","test.bin"))=="current-model";Check(File.Exists(Path.Combine(shared,"audio","test.flac")),"originals exist until verification succeeds");});
   Check(verified&&remaining==0&&File.Exists(Path.Combine(moved,"audio","test.flac")),"verified relocation preserves data");
   Check(File.ReadAllText(Path.Combine(moved,".gitignore")).Contains("*"),"selected storage protects user data from accidental Git staging");
   Check(!Directory.Exists(shared),"old files removed only after a committed relocation");
   Check(SharedStorage.Resolve(next,local,new string[0])==moved,"all releases use saved storage location");
   Check(SharedStorage.Resolve(next,local,new[]{"--data-dir",isolated})==isolated,"explicit test data overrides saved location");
   bool refused=false;try{SharedStorage.Relocate(moved,Path.Combine(moved,"nested"),local,CancellationToken.None,(m,d,t)=>{},p=>{});}catch(IOException){refused=true;}Check(refused,"nested relocation refused");
   string occupied=Path.Combine(root,"occupied");Directory.CreateDirectory(occupied);File.WriteAllText(Path.Combine(occupied,"user.txt"),"keep");refused=false;
   try{SharedStorage.Relocate(moved,occupied,local,CancellationToken.None,(m,d,t)=>{},p=>{});}catch(IOException){refused=true;}Check(refused&&File.ReadAllText(Path.Combine(occupied,"user.txt"))=="keep","occupied destination is never overwritten");
   string failed=Path.Combine(root,"failed-move");refused=false;try{SharedStorage.Relocate(moved,failed,local,CancellationToken.None,(m,d,t)=>{},p=>{throw new IOException("test verification failure");});}catch(IOException){refused=true;}
   Check(refused&&SharedStorage.Resolve(next,local,new string[0])==moved&&File.Exists(Path.Combine(moved,"audio","test.flac")),"failed verification preserves source and saved location");
   Check(!Directory.Exists(failed),"failed copy removes only its own destination files");
   string cancelledTarget=Path.Combine(root,"cancelled-relocation");var moveCancel=new CancellationTokenSource();bool moveCancelled=false;
   try{SharedStorage.Relocate(moved,cancelledTarget,local,moveCancel.Token,(m,d,t)=>moveCancel.Cancel(),p=>{});}catch(OperationCanceledException){moveCancelled=true;}
   Check(moveCancelled&&File.Exists(Path.Combine(moved,"audio","test.flac"))&&SharedStorage.Resolve(next,local,new string[0])==moved,"cancelled relocation preserves original files and pointer");
   Check(!Directory.Exists(cancelledTarget),"cancelled destination can be reused without manual cleanup");
   string missingTarget=Path.Combine(root,"missing-source-target");string oldPointer=File.ReadAllText(Path.Combine(local,"Kavychka-storage.json"));bool verifyCalled=false;refused=false;
   try{SharedStorage.Relocate(shared,missingTarget,local,CancellationToken.None,(m,d,t)=>{},p=>{verifyCalled=true;});}catch(IOException){refused=true;}
   Check(refused&&!verifyCalled&&!Directory.Exists(missingTarget)&&File.ReadAllText(Path.Combine(local,"Kavychka-storage.json"))==oldPointer,"missing default source cannot switch storage pointer or create an empty replacement");
   string unavailable=moved+"-offline";Directory.Move(moved,unavailable);refused=false;try{SharedStorage.Resolve(next,local,new string[0]);}catch(IOException){refused=true;}Check(refused,"unavailable saved disk does not silently download into default storage");Directory.Move(unavailable,moved);
   Console.WriteLine("PASS "+passed+" storage checks");return 0;
  }catch(Exception error){Console.WriteLine("FAIL "+error);return 1;}
 }
}
