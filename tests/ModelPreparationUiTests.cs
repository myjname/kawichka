using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Reflection;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Web.Script.Serialization;
using System.Windows.Forms;

// Exercise the real "Prepare models" action; downloads and jobs are supplied
// by an in-memory backend. Does not install packages or change user settings.
public static class ModelPreparationUiTests {
 static readonly BindingFlags Private=BindingFlags.NonPublic|BindingFlags.Instance;
 static int passed;
 static T Get<T>(object owner,string name){return (T)owner.GetType().GetField(name,Private).GetValue(owner);}
 static void Set(object owner,string name,object value){owner.GetType().GetField(name,Private).SetValue(owner,value);}
 static object Call(object owner,string name,params object[] args){return owner.GetType().GetMethod(name,Private).Invoke(owner,args);}
 static void Check(bool ok,string name){if(!ok)throw new Exception(name);Console.WriteLine("PASS "+name);passed++;}
 sealed class Handler:HttpMessageHandler {
  readonly JavaScriptSerializer json=new JavaScriptSerializer();
  public bool TextGpu,SpeechGpu,WithVoice;
  public readonly List<Dictionary<string,object>> Installs=new List<Dictionary<string,object>>();
  public readonly Dictionary<string,object> Preferences=new Dictionary<string,object>();
  protected override async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request,CancellationToken cancel){
   string route=request.RequestUri.AbsolutePath;object result;
   if(route=="/settings"){
    if(request.Method==HttpMethod.Post){var values=(Dictionary<string,object>)json.DeserializeObject(await request.Content.ReadAsStringAsync());foreach(var entry in values)Preferences[entry.Key]=entry.Value;}
    result=Preferences;
   }else if(route=="/status"){
    result=new{components=new[]{"runtime","cosy-runtime","llama-runtime","tts-cosyvoice3","text-gemma4-12b"}.Select(id=>new{id=id,name=id,installed=id=="runtime",bytes=0}).ToArray(),stats=new{audio_bytes=0},hardware=new{ram_bytes=16000000000L,gpu_name=TextGpu||SpeechGpu?"Test NVIDIA":"",vram_bytes=8000000000L},installation=new{text_gpu=TextGpu,speech_gpu=SpeechGpu}};
   }else if(route=="/library")result=new{rows=new object[0]};
   else if(route=="/jobs"){
    var job=(Dictionary<string,object>)json.DeserializeObject(await request.Content.ReadAsStringAsync());
    Check(Convert.ToString(job["kind"])=="install","model setup submits installation jobs");
    Installs.Add((Dictionary<string,object>)job["args"]);
    result=new{id="install-"+Installs.Count,state="done",message="Ready",done=1,total=1,result=new{installed=true}};
   }else throw new Exception("Unexpected route "+route);
   return new HttpResponseMessage(HttpStatusCode.OK){Content=new StringContent(json.Serialize(result),Encoding.UTF8,"application/json")};
  }
 }
 static async Task Run(){
  foreach(var variant in new[]{new Handler{TextGpu=false,SpeechGpu=false,WithVoice=true},new Handler{TextGpu=false,SpeechGpu=true,WithVoice=true},new Handler{TextGpu=true,SpeechGpu=true,WithVoice=false}}){
   var form=new MainForm();try{
    var settings=Get<Settings>(form,"settings");settings.TextGeneration=true;settings.TextModel="text-gemma4-12b";settings.RuntimeGeneration=false;settings.VoiceProvider=variant.WithVoice?"Qwen local":"System";settings.VoiceModel="tts-cosyvoice3";settings.BackendSettingsMigrated=settings.BackendDailySettingsMigrated=settings.BackendPersonalizationMigrated=true;settings.AutomaticPreparation=false;settings.Voice=false;
    Set(form,"backendStarting",Task.FromResult(0));Set(form,"storagePrepared",true);Set(form,"endpoint","http://127.0.0.1:1");Set(form,"localHttp",new HttpClient(variant));
    await (Task)Call(form,"PrepareSelectedModels");
    Check(variant.Installs.Where(item=>Convert.ToString(item["component"])=="llama-runtime"||Convert.ToString(item["component"]).StartsWith("text-")).All(item=>Convert.ToBoolean(item["gpu"])==variant.TextGpu),"text preparation uses the backend's CPU/CUDA choice");
    Check(variant.Installs.Count==(variant.WithVoice?4:2)&&!variant.Installs.Any(item=>Convert.ToString(item["component"])=="runtime"),"setup downloads only chosen models and missing dependencies");
    Check(variant.Installs.Where(item=>Convert.ToString(item["component"]).StartsWith("tts-")||Convert.ToString(item["component"])=="cosy-runtime").All(item=>Convert.ToBoolean(item["gpu"])==variant.SpeechGpu),"speech preparation uses its independent CPU/CUDA choice");
    Check(!Get<bool>(form,"modelSetupBusy")&&Get<HashSet<string>>(form,"activeJobs").Count==0,"successful setup releases its busy state");
   }finally{Set(form,"exiting",true);Call(form,"CloseLocalAi");Get<NotifyIcon>(form,"tray").Dispose();Get<System.Windows.Forms.Timer>(form,"timer").Dispose();form.Dispose();}
  }
 }
 [STAThread] public static int Main(string[] args){if(!args.Contains("--self-test")||!args.Contains("--data-dir"))return 2;Application.EnableVisualStyles();Application.SetCompatibleTextRenderingDefault(false);int code=1;using(var host=new Form{Opacity=0,ShowInTaskbar=false,Width=1,Height=1}){host.Shown+=async delegate{try{await Run();Console.WriteLine("PASS "+passed+" model setup UI checks");code=0;}catch(Exception e){Console.Error.WriteLine(e);}finally{host.Close();}};Application.Run(host);}return code;}
}
