using System;
using System.Collections.Generic;
using System.IO;
using System.IO.Compression;
using System.Linq;
using System.Speech.Synthesis;
using System.Text;
using System.Threading.Tasks;
using System.Windows.Forms;

public partial class MainForm {
 DayLibraryForm dayLibrary;
 string DayState(bool used,bool rejected,bool approved,bool audio){return used?L.T("Проиграна","Played"):rejected?L.T("Исключена","Excluded"):!approved?L.T("Ждёт отбора","Needs review"):audio?L.T("Готова","Ready"):L.T("Ждёт озвучки","Needs speech");}
 void OpenDayLibrary(){
  if(dayLibrary!=null&&!dayLibrary.IsDisposed){dayLibrary.Activate();return;}
  try{using(var view=CreateDayLibrary()){dayLibrary=view;view.ShowDialog(simpleSettings!=null&&!simpleSettings.IsDisposed?(Form)simpleSettings:this);}}
  catch(Exception error){LogDesktop(error);MessageBox.Show(error.Message,L.T("Дневной список","Daily list"));}finally{dayLibrary=null;}
 }
 DayLibraryForm CreateDayLibrary(){
  bool basic=!settings.TextGeneration&&settings.VoiceProvider=="System";
  var view=new DayLibraryForm(basic?L.T("Готовые фразы · голос Windows. Экспорт в ZIP сохраняет тексты. Для новых формулировок выбери текстовую модель.","Ready-made phrases · Windows voice. ZIP exports contain text. Choose a text model for new wording."):L.T("Новые партии ждут твоего отбора. Готовые записи выдаются один раз; прослушивание их не расходует.","New batches wait for your review. Ready recordings are used once; previews do not consume them."),settings.AutomaticPreparation,settings.AutoKeepDaily,settings.VoiceProvider!="System",!basic);
  view.Progress=()=>localProgress.Text;view.Stop=StopAudio;view.Cancel=async delegate{try{foreach(string id in activeJobs.ToArray())await Api("/jobs/"+id,new{});}catch(Exception error){LogDesktop(error);}};
  view.LoadEntries=async delegate{
   if(basic){var day=await NativeDayWork((store,snapshot,now)=>snapshot.AutomaticPreparation?store.Prepare(snapshot,now,snapshot.AutoKeepDaily):store.Read(snapshot,now));return day.Rows.Where(row=>!row.Deleted&&(row.Active||row.UsedAt.Length>0||row.Rejected)).Select(row=>new DayEntry{Id=row.Id,Category=row.Category,Followup=row.Followup,Text=row.Text,State=DayState(row.UsedAt.Length>0,row.Rejected,row.Approved,true)}).ToList();}
   await RequireDayBackend(false);var result=await Api("/library");return ((object[])result["rows"]).Select(D).Where(row=>S(row,"day")==TodayDesktop&&S(row,"language")==settings.SpeechLanguage).Select(row=>new DayEntry{Id=S(row,"id"),Text=S(row,"text"),Category=S(row,"category"),Followup=B(row,"followup"),State=DayState(S(row,"used_at").Length>0,B(row,"rejected"),B(row,"approved"),S(row,"audio_path").Length>0||settings.VoiceProvider=="System")}).ToList();
  };
  view.Listen=async delegate(string id){
   StopAudio();int version=audioVersion;if(basic){var row=(await NativeDayWork((store,snapshot,now)=>store.Read(snapshot,now))).Rows.First(item=>item.Id==id);if(version!=audioVersion)return;SpeakDayPreview(row.Text);}
   else{await RequireDayBackend(false);var row=((object[])(await Api("/library"))["rows"]).Select(D).First(item=>S(item,"id")==id);if(version!=audioVersion)return;if(S(row,"audio_path").Length>0)await PlayLocal(id,version);else if(settings.VoiceProvider=="System")SpeakDayPreview(S(row,"text"));else throw new InvalidOperationException(L.T("Текст готов, но озвучка ещё не создана. Подготовь недостающее или нажми «Озвучить заново».","The text is ready, but speech is missing. Prepare the day or choose Revoice."));}
  };
  view.SelectEntries=async delegate(string[] ids,string action){if(basic)await NativeDayWork((store,snapshot,now)=>{store.Select(snapshot,now,ids,action);return 0;});else{await RequireDayBackend(false);await Api("/select",new{ids=ids,action=action});}};
  view.OptionsChanged=delegate(bool automatic,bool keep){var draft=Json.Deserialize<Settings>(Json.Serialize(settings));draft.AutomaticPreparation=automatic;draft.AutoKeepDaily=keep;SaveDesktopSettings(draft);return Task.FromResult(0);};
  view.Prepare=async delegate{if(basic)await NativeDayWork((store,snapshot,now)=>store.Prepare(snapshot,now,snapshot.AutoKeepDaily));else{CheckDayWorkAvailable();await RequireDayBackend(true);nextPreparationUtc=DateTime.MinValue;await PrepareDesktopDay(true);}};
  view.Generate=async delegate(int count,string category,bool followup){
   if(basic){await NativeDayWork((store,snapshot,now)=>store.Add(snapshot,now,category,count,followup));return;}
   CheckDayWorkAvailable();await RequireDayBackend(true);string[] stock=null;
   if(!settings.TextGeneration){var batch=await NativeDayWork((store,snapshot,now)=>store.Add(snapshot,now,category,count,followup));stock=batch.Rows.Skip(batch.Rows.Count-count).Select(row=>row.Text).ToArray();}
   await RunJob("batch",new{day=TodayDesktop,count=count,categories=new[]{category},followup=followup,system_text=!settings.TextGeneration,system_voice=settings.VoiceProvider=="System",texts=stock,text_model=settings.RuntimeGeneration?settings.RuntimeTextModel:settings.TextModel});
  };
  view.Revoice=async delegate(string[] ids){CheckDayWorkAvailable();await RequireDayBackend(true);foreach(string id in ids)await RunJob("speech",new{id=id,force=true});};
  view.Export=async delegate(string[] ids){
   if(ids.Length==0)throw new InvalidOperationException(L.T("Список пока пуст.","The list is empty."));
   using(var picker=new SaveFileDialog{Filter="ZIP|*.zip",FileName="Kavychka-"+TodayDesktop+".zip"}){if(picker.ShowDialog(view)!=DialogResult.OK)return;
    nativeTasks++;try{
     byte[] bytes;if(basic)bytes=await NativeDayWork((store,snapshot,now)=>{using(var memory=new MemoryStream()){using(var archive=new ZipArchive(memory,ZipArchiveMode.Create,true)){using(var writer=new StreamWriter(archive.CreateEntry("phrases.json").Open(),new UTF8Encoding(false)))writer.Write(store.Export(snapshot,now,ids));using(var writer=new StreamWriter(archive.CreateEntry("phrases.txt").Open(),new UTF8Encoding(false)))foreach(var row in store.Read(snapshot,now).Rows.Where(row=>ids.Contains(row.Id)))writer.WriteLine(row.Text);}return memory.ToArray();}});
     else{await RequireDayBackend(false);bytes=await ApiBytes("/export",new{ids=ids});}string target=picker.FileName;await Task.Run(()=>SettingsFile.WriteBytes(target,bytes));
    }finally{nativeTasks--;}
   }
  };
  return view;
 }
 void CheckDayWorkAvailable(){if(activeJobs.Count>0||preparationBusy||modelSetupBusy||runtimeInstalling||presetEditing||storageMoving||updateInstalling||nativeTasks>0)throw new InvalidOperationException(L.T("Дождись текущей подготовки или нажми «Отмена».","Wait for the current preparation or choose Cancel."));}
 async Task RequireDayBackend(bool models){
  if(!File.Exists(PythonPath()))throw new InvalidOperationException(L.T("Сначала подготовь выбранные модели на вкладке «Модели». Базовый режим работает без загрузок.","Prepare your selections in Models first. Basic mode works without downloads."));
  await EnsureBackend();await SyncDesktopSettings();if(!models)return;
  var available=((object[])(await Api("/status"))["components"]).Select(D).ToDictionary(row=>S(row,"id"),row=>B(row,"installed"));string textModel=settings.RuntimeGeneration?settings.RuntimeTextModel:settings.TextModel;
  if((settings.TextGeneration&&(!available[textModel]||!available["llama-runtime"]))||(settings.VoiceProvider!="System"&&(!available["runtime"]||!available[settings.VoiceModel]||(settings.VoiceModel=="tts-cosyvoice3"&&!available["cosy-runtime"]))))throw new InvalidOperationException(L.T("Нужные компоненты ещё не готовы. Нажми «Подготовить выбранное» на вкладке «Модели».","The selected components are not ready. Choose Prepare selected models in Models."));
 }
 void SpeakDayPreview(string text){
  EnsureSystemSpeech();
  var voices=systemSpeech.GetInstalledVoices().Where(voice=>voice.Enabled&&voice.VoiceInfo.Culture.TwoLetterISOLanguageName==settings.SpeechLanguage).ToArray();
  if(voices.Length==0)throw new InvalidOperationException(L.T("В Windows нет голоса выбранного языка. Выбери локальную озвучку в «Моделях».","Windows has no voice for the selected language. Choose local speech in Models."));
  systemSpeech.SelectVoice(voices[0].VoiceInfo.Name);if(pet!=null)pet.Speaking=true;activeSystemPrompt=systemSpeech.SpeakAsync(text);
 }
 void EnsureSystemSpeech(){if(systemSpeech!=null)return;systemSpeech=new SpeechSynthesizer();systemSpeech.SpeakCompleted+=delegate(object sender,SpeakCompletedEventArgs e){if(!IsDisposed&&IsHandleCreated)try{BeginInvoke((Action)delegate{if(e.Prompt==activeSystemPrompt&&pet!=null&&!pet.IsDisposed){pet.Speaking=false;pet.Invalidate();}});}catch(InvalidOperationException){}};}
}
