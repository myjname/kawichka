using System;
using System.Collections.Generic;
using System.Drawing;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Net;
using System.Net.Http;
using System.Text;
using CancellationToken=System.Threading.CancellationToken;
using System.Threading.Tasks;
using System.Windows.Forms;

public static class DayLibraryUiTests {
 static readonly BindingFlags Private=BindingFlags.NonPublic|BindingFlags.Instance;
 static int passed;
 static T Get<T>(object owner,string field){return (T)owner.GetType().GetField(field,Private).GetValue(owner);}
 static void Set(object owner,string field,object value){owner.GetType().GetField(field,Private).SetValue(owner,value);}
 static object Call(object owner,string method,params object[] args){return owner.GetType().GetMethod(method,Private).Invoke(owner,args);}
 static IEnumerable<Control> All(Control root){foreach(Control child in root.Controls){yield return child;foreach(var nested in All(child))yield return nested;}}
 static void Check(bool yes,string name){if(!yes)throw new Exception(name);Console.WriteLine("PASS "+name);passed++;}
 static async Task Drain(DayLibraryForm view){var deadline=DateTime.UtcNow.AddSeconds(10);while(Get<bool>(view,"busy")){if(DateTime.UtcNow>=deadline)throw new TimeoutException("Day library UI did not settle");await Task.Delay(10);}}
 static void Close(MainForm form){Set(form,"exiting",true);Call(form,"CloseLocalAi");Get<NotifyIcon>(form,"tray").Dispose();Get<Timer>(form,"timer").Dispose();form.Dispose();}
 static async Task Run(string folder){
  var form=new MainForm();try{
   L.Language="ru";var settings=Get<Settings>(form,"settings");settings.UiLanguage="ru";settings.TimeZoneId=DateTime.UtcNow.Hour<15?"UTC":"Pacific Standard Time";settings.WorkDays=new[]{0,1,2,3,4,5,6};settings.TextGeneration=false;settings.VoiceProvider="System";settings.SpeechLanguage="ru";settings.AutomaticPreparation=true;settings.AutoKeepDaily=false;settings.BackendSettingsMigrated=true;settings.BackendPersonalizationMigrated=true;settings.BackendDailySettingsMigrated=false;settings.BackendSettingsOverrides=new List<string>();
   using(var view=(DayLibraryForm)Call(form,"CreateDayLibrary")){
    var shown=new TaskCompletionSource<bool>();view.Shown+=delegate{shown.TrySetResult(true);};view.Show();await shown.Task;await Drain(view);var entries=await view.LoadEntries();var list=Get<ListView>(view,"list");
    Check(entries.Count>0&&entries.All(row=>row.State=="Ждёт отбора")&&list.Items.Count==entries.Count,"modern day list shows actual native review candidates");
    list.TopItem=list.Items[Math.Min(5,list.Items.Count-1)];string topId=((DayEntry)list.TopItem.Tag).Id;await (Task)Call(view,"Reload");Check(((DayEntry)list.TopItem.Tag).Id==topId,"background refresh preserves the current scroll position");
    var first=list.Items[0];first.Selected=true;string firstId=((DayEntry)first.Tag).Id;All(view).OfType<StudioButton>().Single(button=>button.Text=="Оставить").PerformClick();await Drain(view);
    var stored=new BuiltinDays(folder).Read(settings,DateTime.UtcNow);Check(stored.Rows.Single(row=>row.Id==firstId).Approved&&stored.Rows.Single(row=>row.Id==firstId).UsedAt.Length==0,"Keep updates storage without reserving playback");
    string heard=null;view.Listen=delegate(string id){heard=id;return Task.FromResult(0);};All(view).OfType<StudioButton>().Single(button=>button.Text=="Слушать").PerformClick();await Drain(view);
    Check(heard==firstId&&new BuiltinDays(folder).Read(settings,DateTime.UtcNow).Rows.Single(row=>row.Id==firstId).UsedAt.Length==0,"preview dispatch does not consume a native recording");
    Get<NumericUpDown>(view,"count").Value=3;Get<ComboBox>(view,"category").SelectedIndex=4;Get<CheckBox>(view,"followup").Checked=true;All(view).OfType<StudioButton>().Single(button=>button.Text=="Создать ещё").PerformClick();await Drain(view);
    stored=new BuiltinDays(folder).Read(settings,DateTime.UtcNow);Check(stored.Rows.Count(row=>row.Kind=="manual")==3&&stored.Rows.Where(row=>row.Kind=="manual").All(row=>row.Category=="exercise"&&row.Followup&&!row.Approved),"manual count, category and followup selection create the requested batch");
    typeof(ListView).GetMethod("OnKeyDown",Private).Invoke(list,new object[]{new KeyEventArgs(Keys.Control|Keys.A)});Check(list.SelectedItems.Count==list.Items.Count,"Ctrl+A selects the full daily list for bulk voice and review actions");foreach(ListViewItem item in list.Items)item.Selected=false;
    using(var simple=new SimpleSettingsForm(settings,new List<Dictionary<string,object>>(),false)){
     simple.Saved=draft=>Call(form,"SaveDesktopSettings",draft);Get<CheckBox>(view,"automatic").Checked=false;await Drain(view);Get<CheckBox>(view,"approve").Checked=true;await Drain(view);
     Check(!settings.AutomaticPreparation&&settings.AutoKeepDaily,"daily options update through their real save callback");
     Check((bool)Call(simple,"SaveDraft",false)&&!settings.AutomaticPreparation&&settings.AutoKeepDaily,"saving an already open main form preserves changed daily options");
    }
    var persisted=new System.Web.Script.Serialization.JavaScriptSerializer().Deserialize<Settings>(File.ReadAllText(Path.Combine(folder,"settings.json")));Check(!persisted.AutomaticPreparation&&persisted.AutoKeepDaily&&persisted.BackendSettingsOverrides.Contains("auto_daily")&&persisted.BackendSettingsOverrides.Contains("daily_auto_keep"),"native daily options persist before backend migration");
    view.Size=view.MinimumSize;view.PerformLayout();var batch=Get<ComboBox>(view,"category").Parent;Check(batch.Controls.Cast<Control>().Where(control=>control.Visible).All(control=>control.Right<=batch.ClientSize.Width),"generation and cancel controls fit at the minimum window width");
    Check(list.Columns.Cast<ColumnHeader>().Sum(column=>column.Width)<=list.ClientSize.Width,"daily phrases fit the list without horizontal scrolling");
    using(var image=new Bitmap(view.Width,view.Height)){view.DrawToBitmap(image,new Rectangle(0,0,image.Width,image.Height));image.Save(Path.Combine(folder,"day-library.png"));}
    view.Close();
   }
   Check(!Directory.Exists(Path.Combine(folder,"runtime"))&&!File.Exists(Path.Combine(folder,"library.sqlite3")),"native day review and generation do not bootstrap Python");
   settings.UiLanguage="en";settings.SpeechLanguage="en";settings.AutomaticPreparation=true;settings.AutoKeepDaily=false;L.Language="en";
   using(var english=(DayLibraryForm)Call(form,"CreateDayLibrary")){var shown=new TaskCompletionSource<bool>();english.Shown+=delegate{shown.SetResult(true);};english.Show();await shown.Task;await Drain(english);var rows=await english.LoadEntries();Check(english.Text=="Today's phrases"&&rows.Count>0&&rows.All(row=>row.State=="Needs review"),"English day review uses English controls and status text");english.Close();}
   Directory.CreateDirectory(Path.Combine(folder,"runtime"));File.WriteAllText(Path.Combine(folder,"runtime","python.exe"),"not executed");var handler=new LibraryHandler(RitualSchedule.Local(settings,DateTime.UtcNow).ToString("yyyy-MM-dd"));
   Set(form,"backendStarting",Task.FromResult(0));Set(form,"endpoint","http://127.0.0.1:1");Set(form,"localHttp",new HttpClient(handler));settings.BackendSettingsMigrated=true;settings.BackendPersonalizationMigrated=true;settings.BackendDailySettingsMigrated=true;settings.TextGeneration=true;settings.VoiceProvider="System";settings.RuntimeGeneration=true;settings.RuntimeTextModel="text-qvikhr3-8b";
   using(var enhanced=(DayLibraryForm)Call(form,"CreateDayLibrary")){
    var rows=await enhanced.LoadEntries();Check(rows.Count==1&&rows[0].Id=="generated"&&rows[0].State=="Needs review","modern model library loads current-day backend candidates");
    await enhanced.Generate(2,"break",true);Check(Convert.ToString(handler.LastArgs["text_model"])=="text-qvikhr3-8b"&&Convert.ToBoolean(handler.LastArgs["system_voice"])&&Convert.ToBoolean(handler.LastArgs["followup"]),"manual model batch uses the visible runtime selection and Windows speech mode");
    await enhanced.SelectEntries(new[]{"generated"},"keep");Check(Convert.ToString(handler.LastSelection["action"])=="keep"&&((object[])handler.LastSelection["ids"])[0].ToString()=="generated","model review routes the selected IDs to the backend");
   }
   settings.TextGeneration=false;settings.VoiceProvider="Qwen local";settings.VoiceModel="tts-cosyvoice3";using(var voiceOnly=(DayLibraryForm)Call(form,"CreateDayLibrary")){await voiceOnly.Generate(2,"exercise",true);Check(Convert.ToBoolean(handler.LastArgs["system_text"])&&!Convert.ToBoolean(handler.LastArgs["system_voice"])&&((object[])handler.LastArgs["texts"]).Length==2,"stock text plus local voice submits the requested text batch without a text model");}
  }finally{Close(form);}
 }
 sealed class LibraryHandler:HttpMessageHandler {
  readonly System.Web.Script.Serialization.JavaScriptSerializer json=new System.Web.Script.Serialization.JavaScriptSerializer();readonly string day;
  public Dictionary<string,object> LastArgs,LastSelection;
  public LibraryHandler(string day){this.day=day;}
  protected override async Task<HttpResponseMessage> SendAsync(HttpRequestMessage request,CancellationToken cancel){object reply;string route=request.RequestUri.AbsolutePath;
   if(route=="/settings")reply=new{};
   else if(route=="/status")reply=new{components=new[]{"runtime","cosy-runtime","llama-runtime","tts-cosyvoice3","text-qvikhr3-8b"}.Select(id=>new{id=id,installed=true,name=id,bytes=0}).ToArray(),stats=new{audio_bytes=0}};
   else if(route=="/library")reply=new{rows=new[]{new{id="generated",text="A generated candidate for a short break.",category="break",language="en",day=day,model="system",approved=false,followup=false,audio_path=(string)null,used_at=(string)null,voice_style=""}}};
   else if(route=="/jobs"){var body=(Dictionary<string,object>)json.DeserializeObject(await request.Content.ReadAsStringAsync());LastArgs=(Dictionary<string,object>)body["args"];reply=new{id="job",state="done",done=2,total=2,message="Ready",result=new{count=2}};}
   else if(route=="/select"){LastSelection=(Dictionary<string,object>)json.DeserializeObject(await request.Content.ReadAsStringAsync());reply=new{ok=true};}
   else throw new Exception("Unexpected library request "+route);
   return new HttpResponseMessage(HttpStatusCode.OK){Content=new StringContent(json.Serialize(reply),Encoding.UTF8,"application/json")};
  }
 }
 [STAThread] public static int Main(string[] args){int index=Array.IndexOf(args,"--data-dir");if(index<0||!args.Contains("--self-test"))return 2;string folder=Path.GetFullPath(args[index+1]);Application.EnableVisualStyles();Application.SetCompatibleTextRenderingDefault(false);int code=1;using(var host=new Form{Opacity=0,ShowInTaskbar=false}){host.Shown+=async delegate{try{await Run(folder);Console.WriteLine("PASS "+passed+" daily library UI checks");code=0;}catch(Exception error){Console.Error.WriteLine(error);}finally{host.Close();}};Application.Run(host);}return code;}
}
