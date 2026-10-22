using System;
using System.Collections.Generic;
using System.Drawing;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Speech.Synthesis;
using System.Threading;
using System.Threading.Tasks;
using System.Windows.Forms;

public partial class MainForm {
 PetForm pet;
 PetMessage petMessage;
 NativeActivity activity;
 StudioThemeWatcher themeWatcher;
 DesktopPowerWatcher powerWatcher;
 ReminderCycle cycle;
 SpeechSynthesizer systemSpeech;
 Prompt activeSystemPrompt;
 bool desktopBusy,preparationBusy,desktopReady,legacyDesktopSettings,messagePending,manualMessage,wasFullscreen,storageDeferred;
 bool updateChecking,updateInstalling,modelSetupBusy,modelSetupCancelled,presetEditing,runtimeGenerationBusy;
 CancellationTokenSource updateCancel;
 int updatePercent;
 AppRelease availableRelease;
 DateTime nextUpdateCheckUtc=DateTime.MinValue;
 DateTime nextPreparationUtc=DateTime.MinValue;
 DateTime petHappyUntilUtc=DateTime.MinValue;
 string preparationStatus="";
 Dictionary<string,object> detectedHardware,installationDevices;
 SimpleSettingsForm simpleSettings;
 BuiltinDays builtinDays;
 string builtinRoot;
 int nativeTasks;
 BuiltinDays BasicDays {get{if(builtinDays==null||builtinRoot!=Folder){builtinRoot=Folder;builtinDays=new BuiltinDays(Folder);}return builtinDays;}}
 async Task<T> NativeDayWork<T>(Func<BuiltinDays,Settings,DateTime,T> action){var store=BasicDays;var snapshot=Json.Deserialize<Settings>(Json.Serialize(settings));var now=DateTime.UtcNow;nativeTasks++;try{return await Task.Run(()=>action(store,snapshot,now));}finally{nativeTasks--;}}
 string TodayDesktop {get{return RitualSchedule.Local(settings,DateTime.UtcNow).ToString("yyyy-MM-dd");}}
 bool TestMode {get{return Environment.GetCommandLineArgs().Any(a=>a=="--preview"||a=="--self-test"||a=="--setup-smoke"||a=="--v2-preview"||a=="--health-check");}}

 void InitializeDesktop(){
  if(TestMode)return;
  themeWatcher=new StudioThemeWatcher(this);powerWatcher=new DesktopPowerWatcher(this,HandleDesktopPower);activity=new NativeActivity(settings);pet=new PetForm();pet.RenderingFailed=LogDesktop;
  pet.ClientSize=new Size(settings.PetSize,(int)(settings.PetSize*170/150.0));
  var area=Screen.PrimaryScreen.WorkingArea;pet.Location=settings.PetX==int.MinValue?new Point(area.Right-pet.Width-25,area.Bottom-pet.Height-25):new Point(settings.PetX,settings.PetY);pet.ClampToScreen();
  ApplyVisual();pet.Clicked=ShowPetMenu;pet.PositionChanged=delegate{settings.PetX=pet.Left;settings.PetY=pet.Top;Save();if(petMessage!=null&&!petMessage.IsDisposed)petMessage.AnchorTo(pet);};
  FormClosed+=delegate{if(themeWatcher!=null)themeWatcher.Dispose();if(powerWatcher!=null)powerWatcher.Dispose();if(activity!=null)activity.Dispose();if(petMessage!=null)petMessage.Dispose();if(pet!=null)pet.Dispose();if(systemSpeech!=null)systemSpeech.Dispose();if(simpleSettings!=null)simpleSettings.Dispose();};
  FormClosing+=delegate{PersistDesktopReminder(DateTime.UtcNow);};
 }
 async Task DesktopStartup(){
  if(TestMode)return;
  if(settings.RunAtLogin&&!SharedStorage.IsIsolated(Environment.GetCommandLineArgs()))try{var entry=StartupIntegration.Read(StartupIntegration.Folder);if(entry!=null&&!string.Equals(entry.Target,Application.ExecutablePath,StringComparison.OrdinalIgnoreCase))ApplyStartupPreference(true);}catch(Exception error){LogDesktop(error);preparationStatus=L.T("Не удалось обновить ярлык автозапуска.","Could not update the startup shortcut.");}
  if(!settings.SetupComplete)OpenSimpleSettings();
  try{await PrepareStorage();}catch(OperationCanceledException){storageDeferred=true;preparationStatus=L.T("Перенос отменён. Базовые напоминания работают; подготовку можно продолжить кнопкой в «Моделях».","Migration cancelled. Basic reminders work; continue setup using the button in Models.");}
  if(!storageDeferred&&File.Exists(PythonPath())&&(settings.VoiceProvider!="System"||settings.TextGeneration||legacyDesktopSettings||File.Exists(Path.Combine(Folder,"library.sqlite3")))){
   try{await EnsureBackend();await LoadLocalSettings();await RefreshComponents();await RefreshLibrary();}catch(Exception error){preparationStatus=L.T("Локальная генерация пока недоступна. Работают запасные фразы.","Local generation is unavailable. Ready-made reminders still work.");LogDesktop(error);}
  }
  desktopReady=true;dailyTimer.Start();if(pet!=null&&settings.PetVisible)pet.Show();
  Hide();
  RestoreDesktopReminder(DateTime.UtcNow);
  await PrepareDesktopDay();
  if(settings.AutomaticUpdates)QueueUpdateCheck();
 }
 void OpenSimpleSettings(){
  if(simpleSettings!=null&&!simpleSettings.IsDisposed){simpleSettings.Activate();return;}
  L.Language=settings.UiLanguage;
  simpleSettings=new SimpleSettingsForm(settings,PersonalPresets.List(Folder),!settings.SetupComplete);
  simpleSettings.Saved=SaveDesktopSettings;
  simpleSettings.ThemeChanged=delegate(string mode){string previous=settings.ThemeMode;settings.ThemeMode=mode;if(!Save()){settings.ThemeMode=previous;throw new IOException(settingsSaveError);}};
  simpleSettings.Advanced=delegate{simpleSettings.Close();Restore();};
  simpleSettings.Prepare=async delegate{try{await PrepareSelectedModels();}catch(OperationCanceledException){preparationStatus=L.T("Подготовка отменена. Уже загруженные компоненты сохранены.","Preparation cancelled. Downloaded components are kept.");}catch(Exception e){LogDesktop(e);preparationStatus=e.Message;MessageBox.Show(simpleSettings,e.Message,L.T("Подготовка моделей","Model preparation"));}finally{if(simpleSettings!=null&&!simpleSettings.IsDisposed)RefreshSimpleStatus(simpleSettings);}};
  simpleSettings.CancelPreparation=async delegate{if(updateInstalling){if(updateCancel!=null)updateCancel.Cancel();return;}modelSetupCancelled=true;if(bootstrapCancel!=null)bootstrapCancel.Cancel();foreach(string id in activeJobs.ToArray())try{await Api("/jobs/"+id,new{});}catch(Exception e){LogDesktop(e);}preparationStatus=L.T("Останавливаю подготовку…","Stopping preparation…");};
  simpleSettings.ChangeStorage=async delegate{try{if(preparationBusy||modelSetupBusy||updateInstalling||storagePreparing!=null||nativeTasks>0)throw new InvalidOperationException(L.T("Сначала дождись подготовки или отмени её.","Wait for preparation or cancel it first."));await ChangeStorage();}catch(Exception e){LogDesktop(e);MessageBox.Show(simpleSettings,e.Message,L.T("Хранилище","Storage"));}};
  simpleSettings.Breakfast=DesktopBreakfast;simpleSettings.Stretch=async delegate{await StartDesktopReminder("exercise",false);};
  simpleSettings.DayLibrary=OpenDayLibrary;
  simpleSettings.RefreshStatus=RefreshSimpleStatus;RefreshSimpleStatus(simpleSettings);
  simpleSettings.Import=ImportDesktopPreset;
  simpleSettings.ManagePreset=ManageDesktopPreset;
  simpleSettings.Updates=async delegate{await CheckDesktopUpdates(true);};
  simpleSettings.Show();
 }
 void RefreshSimpleStatus(SimpleSettingsForm view){
  string next;
  if(settings.Paused)next=settings.PauseUntilUtc.HasValue?L.T("На паузе до ","Paused until ")+RitualSchedule.Local(settings,settings.PauseUntilUtc.Value).ToString("dd.MM HH:mm"):L.T("На паузе до включения вручную","Paused until resumed manually");
  else if(messagePending)next=L.T("Сообщение у маскота ждёт твоего ответа","The mascot is waiting for your response");
  else{var e=RitualSchedule.Next(settings,DateTime.UtcNow);next=e==null?L.T("В расписании пока нет напоминаний","No reminders scheduled"):RitualSchedule.Local(settings,e.Utc).ToString("dd.MM · HH:mm")+"  ·  "+L.Category(e.Category);}
  if(activity!=null&&!activity.Available)next+=Environment.NewLine+L.T("Проверка активности недоступна","Activity monitoring is unavailable");
  if(pet!=null&&!string.IsNullOrEmpty(pet.RenderingError))next+=Environment.NewLine+L.T("Не удалось обновить изображение маскота; сообщения доступны.","Could not refresh the mascot image; messages remain available.");
  bool busy=modelSetupBusy||preparationBusy||runtimeInstalling||activeJobs.Count>0||updateInstalling||storagePreparing!=null||storageMoving;
  string detail=busy&&!updateInstalling?localProgress.Text:preparationStatus;
  if(string.IsNullOrWhiteSpace(detail))detail=L.T("Напоминания работают. Готовые модели повторно не скачиваются.","Reminders are active. Installed models are reused.");
  view.UpdateStatus(next,detail,busy,updateInstalling?updatePercent:localBar.Value,!updateInstalling&&localBar.Style==ProgressBarStyle.Marquee,Folder);view.UpdateHardware(detectedHardware,installationDevices);view.UpdateVersion(availableRelease==null?"":availableRelease.Version.ToString(),updateChecking||updateInstalling);
 }
 void SaveDesktopSettings(Settings draft){
  var previous=Json.Deserialize<Settings>(Json.Serialize(settings));bool startupChanged=settings.RunAtLogin!=draft.RunAtLogin;
  if(settings.RunAtLogin!=draft.RunAtLogin)ApplyStartupPreference(draft.RunAtLogin);
  if(!settings.BackendSettingsMigrated){
   if(settings.BackendSettingsOverrides==null)settings.BackendSettingsOverrides=new List<string>();
   var chosen=settings.BackendSettingsOverrides;
   if(settings.TextModel!=draft.TextModel)chosen.Add("text_model");
   if(settings.RuntimeTextModel!=draft.RuntimeTextModel)chosen.Add("runtime_text_model");
   if(settings.VoiceModel!=draft.VoiceModel||settings.VoiceProvider!=draft.VoiceProvider)chosen.Add("tts_model");
   if(settings.Swear!=draft.Swear)chosen.Add("swear");
   if(settings.RuntimeGeneration!=draft.RuntimeGeneration)chosen.Add("runtime");
   if(settings.TextGeneration!=draft.TextGeneration||settings.RuntimeGeneration!=draft.RuntimeGeneration)chosen.Add(draft.RuntimeGeneration?"runtime_text_model":"text_model");
   settings.BackendSettingsOverrides=chosen.Distinct().ToList();
  }
  if(!settings.BackendDailySettingsMigrated){if(settings.BackendSettingsOverrides==null)settings.BackendSettingsOverrides=new List<string>();if(settings.AutomaticPreparation!=draft.AutomaticPreparation)settings.BackendSettingsOverrides.Add("auto_daily");if(settings.AutoKeepDaily!=draft.AutoKeepDaily)settings.BackendSettingsOverrides.Add("daily_auto_keep");settings.BackendSettingsOverrides=settings.BackendSettingsOverrides.Distinct().ToList();}
  if(settings.CharacterFile!=draft.CharacterFile)settings.BackendPersonalizationMigrated=true;
  bool voiceChanged=settings.Voice!=draft.Voice||settings.VoiceProvider!=draft.VoiceProvider||settings.VoiceProfile!=draft.VoiceProfile||settings.SpeechLanguage!=draft.SpeechLanguage;
  bool visualChanged=settings.VisualFile!=draft.VisualFile||settings.PetSize!=draft.PetSize;
  string[] runtime={"History","HandledEvents","LastBreak","LastMeal","ExerciseUtc","PauseUntilUtc","Paused","PetX","PetY","LastNotifiedUpdate","BackendPersonalizationMigrated","BackendSettingsMigrated","BackendDailySettingsMigrated","BackendSettingsOverrides","PersonalizationMigrated"};
  foreach(var field in typeof(Settings).GetFields())if(!runtime.Contains(field.Name))field.SetValue(settings,field.GetValue(draft));
  settings.SchemaVersion=2;L.Language=settings.UiLanguage;
  if(!settings.ExerciseEnabled)settings.ExerciseUtc=null;
  if(!Save()){
   string failure=settingsSaveError;foreach(var field in typeof(Settings).GetFields())field.SetValue(settings,field.GetValue(previous));L.Language=settings.UiLanguage;
   if(startupChanged)try{ApplyStartupPreference(previous.RunAtLogin);}catch(Exception error){LogDesktop(error);failure+=Environment.NewLine+L.T("Не удалось вернуть прежний автозапуск: ","Could not restore the previous startup preference: ")+error.Message;}
   throw new IOException(failure);
  }
  RefreshSharedModelControls();if(!settings.ExerciseEnabled)CancelDesktopExercise();if(visualChanged)ApplyVisual();
  nextPreparationUtc=DateTime.MinValue;if(voiceChanged)StopAudio();
  if(endpoint!=null)QueueDesktopSync();
 }
 void ApplyVisual(){if(pet==null)return;pet.ClientSize=new Size(settings.PetSize,(int)(settings.PetSize*170/150.0));try{pet.SetVisual(PersonalPresets.FilePath(Folder,settings.VisualFile,"visual"),settings.VisualFile.Length==0?"auto":Convert.ToString(PersonalPresets.Read(Folder,settings.VisualFile,"visual")["layout"]));}catch(Exception e){LogDesktop(e);pet.SetVisual("");}pet.ClampToScreen();}
 void ShowPetMenu(){
  var menu=new ContextMenuStrip();StudioColors.PrepareMenu(menu);
  menu.Items.Add(L.T("Настройки","Settings"),null,delegate{OpenSimpleSettings();});
  menu.Items.Add(L.T("Фразы на сегодня","Today's phrases"),null,delegate{OpenDayLibrary();});
  menu.Items.Add(L.T("Я позавтракала / позавтракал","I have had breakfast"),null,delegate{DesktopBreakfast();});
  menu.Items.Add(L.T("Размяться сейчас","Stretch now"),null,async delegate{await StartDesktopReminder("exercise",false);});
  menu.Items.Add(L.T("Проверить голос","Test voice"),null,async delegate{await StartDesktopReminder("break",false);});
  var pauses=new ToolStripMenuItem(L.T("Пауза","Pause"));foreach(int minutes in new[]{15,30,60}){int value=minutes;pauses.DropDownItems.Add(minutes+L.T(" минут"," minutes"),null,delegate{SetDesktopPause(value);});}
  pauses.DropDownItems.Add(L.T("До завтра","Until tomorrow"),null,delegate{SetDesktopPause(-1);settings.PauseUntilUtc=RitualSchedule.Utc(settings,RitualSchedule.Local(settings,DateTime.UtcNow).Date.AddDays(1));Save();});
  pauses.DropDownItems.Add(L.T("До включения вручную","Until resumed manually"),null,delegate{SetDesktopPause(-1);});menu.Items.Add(pauses);
  if(settings.Paused)menu.Items.Add(L.T("Освободить Кавычку","Release Kavychka"),null,delegate{settings.Paused=false;settings.PauseUntilUtc=null;cycle=null;Save();});
  menu.Items.Add(preparationStatus.Length==0?L.T("Готовые напоминания доступны","Ready-made reminders available"):preparationStatus).Enabled=false;
  menu.Items.Add(L.T("Повторить подготовку","Retry preparation"),null,async delegate{nextPreparationUtc=DateTime.MinValue;await PrepareDesktopDay(true);});
  menu.Items.Add(availableRelease==null?L.T("Проверить обновления","Check for updates"):L.T("Обновить до ","Update to ")+availableRelease.Version,null,async delegate{await CheckDesktopUpdates(true);});
  menu.Items.Add(L.T("Расширенные настройки","Advanced settings"),null,delegate{Restore();});
  menu.Items.Add(L.T("Выход","Exit"),null,delegate{exiting=true;Close();});
  menu.Closed+=delegate{menu.Dispose();};menu.Show(pet,new Point(0,pet.Height));
 }
 void SetDesktopPause(int minutes){settings.Paused=true;settings.PauseUntilUtc=minutes<0?(DateTime?)null:DateTime.UtcNow.AddMinutes(minutes);cycle=null;messagePending=false;manualMessage=false;StopAudio();if(petMessage!=null)petMessage.Hide();Save();PersistDesktopReminder(DateTime.UtcNow);}
 void DesktopBreakfast(){if(!settings.ExerciseEnabled){OpenSimpleSettings();return;}settings.ExerciseUtc=DateTime.UtcNow.AddMinutes(settings.ExerciseAfter);Save();UpdateLabels();preparationStatus=L.T("Разминка в ","Stretch at ")+RitualSchedule.Local(settings,settings.ExerciseUtc.Value).ToString("HH:mm");if(pet!=null){petHappyUntilUtc=DateTime.UtcNow.AddSeconds(4);pet.Happy=true;pet.Invalidate();}}
 async Task DesktopTick(){
  if(TestMode||!desktopReady||desktopBusy||storageMoving||storagePreparing!=null)return;desktopBusy=true;
  try{
   var now=DateTime.UtcNow;if(pet!=null&&pet.Happy&&now>=petHappyUntilUtc)pet.Happy=false;bool fullscreen=FullscreenDetector.IsFullscreen();
   if(now>=nextReminderCheckpointUtc)PersistDesktopReminder(now);
   if(fullscreen&&!wasFullscreen){if(settings.FullscreenAudio=="silent"||settings.FullscreenAudio=="chime")StopAudio();else if(settings.FullscreenAudio=="voice")NotificationChime.Stop();}wasFullscreen=fullscreen;
   if(settings.AutomaticUpdates&&now>=nextUpdateCheckUtc&&!updateChecking&&!updateInstalling)QueueUpdateCheck();
   DateTime? resumedAt=null;
   if(settings.Paused&&settings.PauseUntilUtc.HasValue&&now>=settings.PauseUntilUtc.Value){resumedAt=settings.PauseUntilUtc;settings.Paused=false;settings.PauseUntilUtc=null;Save();}
   if(pet!=null){pet.Paused=settings.Paused;pet.PauseText=settings.PauseUntilUtc.HasValue?Math.Max(0,(int)Math.Ceiling((settings.PauseUntilUtc.Value-now).TotalMinutes))+L.T(" мин"," min"):L.T("На паузе","Paused");bool visible=settings.PetVisible&&(!fullscreen||settings.FullscreenVisual=="both");if(visible&&!pet.Visible)pet.Show();if(!visible&&pet.Visible)pet.Hide();if(petMessage!=null){if((settings.Paused&&!manualMessage)||(fullscreen&&settings.FullscreenVisual=="none"))petMessage.Hide();else if(messagePending&&!petMessage.Visible)petMessage.Show();}}
   var due=RitualSchedule.Due(settings,now);
   if(resumedAt.HasValue){var missed=due.Where(e=>e.Utc<resumedAt.Value).ToArray();RitualSchedule.MarkHandled(settings,missed);if(missed.Any(e=>e.Category=="exercise"))settings.ExerciseUtc=null;due=due.Where(e=>e.Utc>=resumedAt.Value).ToList();if(missed.Length>0)Save();}
   if(settings.ExerciseUtc.HasValue&&now-settings.ExerciseUtc.Value>TimeSpan.FromMinutes(5)){settings.ExerciseUtc=null;Save();}
   if(settings.Paused){RitualSchedule.MarkHandled(settings,due);if(due.Any(e=>e.Category=="exercise"))settings.ExerciseUtc=null;if(due.Count>0)Save();return;}
   bool idle=activity!=null&&activity.IsIdle(now);
   if(cycle!=null&&!cycle.Finished&&(idle||activity!=null&&activity.HasRestedSince(cycle.StartedUtc))){cycle.Finished=true;PersistDesktopReminder(now);}
   if(await DeliverDueDesktopReminders(due,idle))return;
   if(cycle!=null){bool finished=cycle.Finished;int repeats=cycle.RepeatsSent;bool repeat=cycle.Tick(now,settings,idle);if(finished!=cycle.Finished||repeats!=cycle.RepeatsSent)PersistDesktopReminder(now);if(repeat)await SpeakDesktopReminder(cycle.Category,true);}
   MaybeOfferDesktopUpdate();
  }catch(Exception e){LogDesktop(e);}finally{desktopBusy=false;}
 }
 async Task StartDesktopReminder(string category,bool automatic){
  if(settings.Paused&&automatic)return;
  manualMessage=!automatic;
  cycle=automatic?new ReminderCycle(category,DateTime.UtcNow,settings):null;
  await SpeakDesktopReminder(category,false);
 }
 async Task SpeakDesktopReminder(string category,bool followup){
  StopAudio();int version=audioVersion;string text;
  if(petMessage!=null){petMessage.Close();petMessage=null;}messagePending=false;
  reminderCategory=category;reminderFollowup=followup;reminderMessageUtc=DateTime.UtcNow;reminderText="";PersistDesktopReminder(DateTime.UtcNow);
  try{text=(await NativeDayWork((store,snapshot,now)=>store.Consume(snapshot,now,category,followup))).Text;}catch(Exception error){LogDesktop(error);preparationStatus=L.T("Не удалось сохранить дневной список. Использую запасную фразу.","Could not save the daily list. Using a spare phrase.");text=Fallbacks.Pick(settings,category,followup,Environment.TickCount&0x7FFFFFFF);}
  string audioId=null;
  if(version!=audioVersion)return;lastText.Text=text;Save();
  reminderCategory=category;reminderFollowup=followup;reminderMessageUtc=DateTime.UtcNow;ShowDesktopMessage(text,category,followup);PersistDesktopReminder(DateTime.UtcNow);
  bool full=FullscreenDetector.IsFullscreen();
  bool chime=settings.NotificationSound&&(!full||settings.FullscreenAudio=="all"||settings.FullscreenAudio=="chime");bool speak=settings.Voice&&(!full||settings.FullscreenAudio=="all"||settings.FullscreenAudio=="voice");
  if(chime)NotificationChime.Play();
  // Display the reminder first. Slow local services must not hide it. A late
  // runtime result remains unused in the library for a subsequent reminder.
  Task<Dictionary<string,object>> fresh=settings.RuntimeGeneration?GenerateRuntimeReminder(category,followup,text):Task.FromResult<Dictionary<string,object>>(null);
  var ready=settings.RuntimeGeneration?Task.FromResult<Dictionary<string,object>>(null):ConsumeDesktopReady(category,followup);
  if(chime)await Task.Delay(1200);
  if(settings.RuntimeGeneration&&!fresh.IsCompleted)await Task.WhenAny(fresh,Task.Delay(8000));
  Dictionary<string,object> chosen=fresh.IsCompleted?await fresh:null;
  if(version!=audioVersion)return;
  if(chosen!=null){try{chosen=D((await Api("/claim",new{id=S(chosen,"id"),allow_text=settings.VoiceProvider=="System"}))["row"]);}catch(Exception e){LogDesktop(e);chosen=null;}}
  if(chosen==null){if(settings.RuntimeGeneration)ready=ConsumeDesktopReady(category,followup);if(!ready.IsCompleted)await Task.WhenAny(ready,Task.Delay(1000));if(ready.IsCompleted)chosen=await ready;}
  if(version!=audioVersion)return;
  if(chosen!=null){text=S(chosen,"text");if(S(chosen,"audio_path").Length>0)audioId=S(chosen,"id");try{await NativeDayWork((store,snapshot,now)=>{store.Remember(snapshot,now,text,category,followup);return 0;});}catch(Exception error){LogDesktop(error);}if(version!=audioVersion)return;petMessage.SetMessage(text);if(pet!=null)petMessage.AnchorTo(pet);lastText.Text=text;reminderText=text;PersistDesktopReminder(DateTime.UtcNow);}
  speak=settings.Voice&&(!FullscreenDetector.IsFullscreen()||settings.FullscreenAudio=="all"||settings.FullscreenAudio=="voice");if(!speak)return;
  if(audioId!=null){try{await PlayLocal(audioId,version);return;}catch(Exception e){LogDesktop(e);}}
  if(version!=audioVersion)return;
  try{
   EnsureSystemSpeech();
   systemSpeech.SpeakAsyncCancelAll();var voices=systemSpeech.GetInstalledVoices().Where(v=>v.Enabled&&v.VoiceInfo.Culture.TwoLetterISOLanguageName==settings.SpeechLanguage).ToArray();
   if(voices.Length==0){preparationStatus=L.T("В Windows нет голоса для выбранного языка. Сообщение показано; локальную озвучку можно выбрать в «Моделях».","Windows has no voice for the selected language. The message is shown; choose local speech in Models.");return;}
   systemSpeech.SelectVoice(voices[0].VoiceInfo.Name);
   if(pet!=null)pet.Speaking=true;activeSystemPrompt=systemSpeech.SpeakAsync(text);
  }catch(Exception e){LogDesktop(e);preparationStatus=L.T("Сообщение показано. Системный голос недоступен.","Message shown. System speech is unavailable.");}
 }
 async void FinishPetSpeech(int version,double seconds){await Task.Delay(TimeSpan.FromSeconds(Math.Max(0.1,Math.Min(300,seconds))));if(version==audioVersion&&!IsDisposed&&pet!=null&&!pet.IsDisposed){pet.Speaking=false;pet.Invalidate();}}
 async Task<Dictionary<string,object>> ConsumeDesktopReady(string category,bool followup){
  if(endpoint==null||(!settings.TextGeneration&&settings.VoiceProvider=="System"))return null;
  try{return D((await Api("/consume",new{category=category,day=TodayDesktop,followup=followup,language=settings.SpeechLanguage,allow_text=settings.VoiceProvider=="System"}))["row"]);}catch(Exception e){LogDesktop(e);nextPreparationUtc=DateTime.MinValue;return null;}
 }
 async Task<Dictionary<string,object>> GenerateRuntimeReminder(string category,bool followup,string fallback){
  if(runtimeGenerationBusy||presetEditing||modelSetupBusy||preparationBusy||updateInstalling||storageMoving||storageDeferred||storagePreparing!=null||activeJobs.Count>0||!File.Exists(PythonPath())||(!settings.TextGeneration&&settings.VoiceProvider=="System")||(settings.DeferGenerationFullscreen&&FullscreenDetector.IsFullscreen()))return null;
  runtimeGenerationBusy=true;try{await EnsureBackend();await SyncDesktopSettings();return await RunJob("runtime",new{day=TodayDesktop,category=category,followup=followup,reserve=false,system_text=!settings.TextGeneration,system_voice=settings.VoiceProvider=="System",text=fallback});}
  catch(Exception e){LogDesktop(e);preparationStatus=L.T("Новая запись пока не готова. Использую запасную и продолжу подготовку.","The new recording is not ready. Using a fallback and continuing preparation.");nextPreparationUtc=DateTime.MinValue;return null;}
  finally{runtimeGenerationBusy=false;}
 }
 void LogDesktop(Exception e){try{string logs=Path.Combine(Folder,"logs");Directory.CreateDirectory(logs);File.AppendAllText(Path.Combine(logs,"desktop.log"),DateTime.UtcNow.ToString("o")+" "+e+Environment.NewLine);}catch{}}
 async Task SyncDesktopSettings(){
  if(!settings.BackendSettingsMigrated||!settings.BackendPersonalizationMigrated||!settings.BackendDailySettingsMigrated)MigrateBackendSettings(await Api("/settings"));
  string description=PersonalPresets.Description(Folder,settings.CharacterFile);
  if(description.Length==0)description=PersonalPresets.DefaultDescription(Application.StartupPath,settings.SpeechLanguage);
  var values=new Dictionary<string,object>{{"speech_language",settings.SpeechLanguage},{"voice_profile",settings.VoiceProfile},{"address_form",settings.AddressForm},{"user_name",settings.UserName},{"swear",settings.Swear},{"text_model",settings.TextModel},{"runtime_text_model",settings.RuntimeTextModel},{"tts_model",settings.VoiceModel},{"runtime",settings.RuntimeGeneration},{"auto_daily",settings.AutomaticPreparation},{"daily_auto_keep",settings.AutoKeepDaily}};
  values["character_description"]=description;
  await Api("/settings",values);
 }
 async void QueueDesktopSync(){try{await SyncDesktopSettings();}catch(Exception e){LogDesktop(e);preparationStatus=L.T("Настройки генератора пока не применены. Подготовка повторится.","Generator settings have not been applied yet. Preparation will retry.");}}
 async Task PrepareDesktopDay(bool retry=false){
  if(TestMode||!desktopReady||preparationBusy||updateInstalling||modelSetupBusy||presetEditing||nativeTasks>0||DateTime.UtcNow<nextPreparationUtc||activeJobs.Count>0||runtimeInstalling||storageMoving)return;
  preparationBusy=true;nextPreparationUtc=DateTime.UtcNow.AddSeconds(30);
  try{
   if(endpoint!=null&&audioCleanupDay!=TodayDesktop){await RunJob("expire-audio",new{day=TodayDesktop});audioCleanupDay=TodayDesktop;}
   if(!settings.TextGeneration&&settings.VoiceProvider=="System"){
    if(settings.AutomaticPreparation||retry){var basic=await NativeDayWork((store,snapshot,now)=>store.Prepare(snapshot,now,snapshot.AutoKeepDaily));preparationStatus=L.T("Готовые фразы на сегодня: ","Ready-made phrases for today: ")+basic.Rows.Count(row=>row.Active&&!row.Deleted&&!row.Rejected)+L.T(". Новые формулировки доступны с текстовой моделью.",". Choose a text model for new wording.");}return;
   }
   if(storageDeferred)return;
   if(settings.DeferGenerationFullscreen&&FullscreenDetector.IsFullscreen())return;
   if(!File.Exists(PythonPath()))return;
   await EnsureBackend();await SyncDesktopSettings();var statusResult=await Api("/status");var available=((object[])statusResult["components"]).Select(D).ToDictionary(c=>S(c,"id"),c=>B(c,"installed"));
   string activeTextModel=settings.RuntimeGeneration?settings.RuntimeTextModel:settings.TextModel;
   if(settings.TextGeneration&&(!available[activeTextModel]||!available["llama-runtime"]))return;
   if(settings.VoiceProvider!="System"&&(!available["runtime"]||!available[settings.VoiceModel]||(settings.VoiceModel=="tts-cosyvoice3"&&!available["cosy-runtime"])))return;
   if(settings.RuntimeGeneration){if(settings.VoiceProvider!="System")await RunJob("repair",new{day=TodayDesktop,retry=retry});return;}
   var plan=RitualSchedule.Preparation(settings,DateTime.UtcNow);
   var tasks=plan.Select((slot,index)=>new{slot=slot.Id,category=slot.Category,followup=slot.Followup,text=Fallbacks.Pick(settings,slot.Category,slot.Followup,index,false)}).ToArray();
   var result=await RunJob("prepare",new{day=TodayDesktop,tasks=tasks.Take(1000).ToArray(),retry=retry,automatic=!retry,system_text=!settings.TextGeneration,system_voice=settings.VoiceProvider=="System"});
   if(result!=null)preparationStatus=B(result,"disabled")?L.T("Автоподготовка выключена. Готовые записи и запасные фразы доступны.","Automatic preparation is off. Prepared and ready-made reminders are available."):L.T("Готово ","Ready ")+result["ready"]+L.T(" из "," of ")+result["total"]+L.T(" записей"," recordings");
  }catch(Exception e){LogDesktop(e);preparationStatus=L.T("Работают запасные фразы. Подготовку повторю.","Ready-made phrases work. Preparation will retry.");nextPreparationUtc=DateTime.UtcNow.AddMinutes(2);}finally{preparationBusy=false;}
 }
 async Task PrepareSelectedModels(){
  if(activeJobs.Count>0||preparationBusy||modelSetupBusy||updateInstalling||nativeTasks>0)throw new InvalidOperationException(L.T("Дождись текущей подготовки или отмени её в расширенных настройках.","Wait for preparation or cancel it in Advanced."));
  modelSetupCancelled=false;modelSetupBusy=true;try{
  if(!settings.TextGeneration&&settings.VoiceProvider=="System"){MessageBox.Show(L.T("Всё готово. Для этого режима загрузки не нужны.","Ready. This mode needs no downloads."));return;}
  storageDeferred=false;await PrepareStorage();await EnsureBackend();
  CheckModelPreparationCancelled();
  var statusResult=await Api("/status");ReadHardwareStatus(statusResult);var devices=installationDevices;
  bool speechGpu=B(devices,"speech_gpu"),textGpu=B(devices,"text_gpu");
  var available=((object[])statusResult["components"]).Select(D).ToDictionary(c=>S(c,"id"),c=>B(c,"installed"));
  if(settings.VoiceProvider!="System"){
   if(!available["runtime"])await InstallRuntime(speechGpu);CheckModelPreparationCancelled();
   if(settings.VoiceModel=="tts-cosyvoice3"&&!available["cosy-runtime"])await RunJob("install",new{component="cosy-runtime",gpu=speechGpu});CheckModelPreparationCancelled();
   await RunJob("install",new{component=settings.VoiceModel,gpu=speechGpu});
  }
  CheckModelPreparationCancelled();
  if(settings.TextGeneration){await RunJob("install",new{component="llama-runtime",gpu=textGpu});CheckModelPreparationCancelled();await RunJob("install",new{component=settings.RuntimeGeneration?settings.RuntimeTextModel:settings.TextModel,gpu=textGpu});}
  CheckModelPreparationCancelled();
  await SyncDesktopSettings();nextPreparationUtc=DateTime.MinValue;
  }finally{modelSetupBusy=false;}await PrepareDesktopDay();
 }
 void CheckModelPreparationCancelled(){if(modelSetupCancelled)throw new OperationCanceledException();}
 bool CanManagePresets(){if(preparationBusy||modelSetupBusy||presetEditing||updateInstalling||storagePreparing!=null||activeJobs.Count>0){MessageBox.Show(simpleSettings,L.T("Дождись текущей подготовки или отмени её перед изменением пресета.","Wait for preparation or cancel it before changing a preset."));return false;}return true;}
 void ManageDesktopPreset(string kind,string id){
  if(!CanManagePresets())return;
  presetEditing=true;try{using(var editor=new PresetEditorForm(Folder,Application.StartupPath,kind,id)){
   if(editor.ShowDialog(simpleSettings)!=DialogResult.OK)return;
   if(editor.Deleted){if(kind=="voice"&&settings.VoiceProfile==id)settings.VoiceProfile="";if(kind=="character"&&settings.CharacterFile==id)settings.CharacterFile="";if(kind=="visual"&&settings.VisualFile==id){settings.VisualFile="";ApplyVisual();}Save();}
   if(simpleSettings!=null&&!simpleSettings.IsDisposed)simpleSettings.ReloadPresets(PersonalPresets.List(Folder),kind,editor.ResultId);
   if(endpoint!=null)QueueDesktopSync();
  }}catch(Exception e){LogDesktop(e);MessageBox.Show(simpleSettings,e.Message,L.T("Пресет","Preset"));}finally{presetEditing=false;}
 }
 void ImportDesktopPreset(string kind){
  if(!CanManagePresets())return;
  presetEditing=true;try{
  using(var picker=new OpenFileDialog{Multiselect=kind=="voice",Filter=kind=="voice"?L.T("Аудиофайлы и пресеты|*.*","Audio files and presets|*.*"):kind=="character"?L.T("Описание или пресет|*.txt;*.kavychka","Description or preset|*.txt;*.kavychka"):L.T("Изображение или пресет|*.png;*.webp;*.gif;*.jpg;*.jpeg;*.kavychka","Image or preset|*.png;*.webp;*.gif;*.jpg;*.jpeg;*.kavychka")}){
   if(picker.ShowDialog(simpleSettings)!=DialogResult.OK)return;
   try{string id;using(var progress=new StudioProgressForm(L.T("Импорт пресета","Import preset"),(token,message)=>PersonalPresets.Import(Folder,kind,picker.FileNames,"",token,message))){progress.ShowDialog(simpleSettings);if(progress.Error is OperationCanceledException)return;if(progress.Error!=null)throw progress.Error;id=progress.ResultId;}
    using(var editor=new PresetEditorForm(Folder,Application.StartupPath,kind,id)){if(editor.ShowDialog(simpleSettings)==DialogResult.OK&&!editor.Deleted){if(simpleSettings!=null&&!simpleSettings.IsDisposed)simpleSettings.ReloadPresets(PersonalPresets.List(Folder),kind,editor.ResultId);}else if(!editor.Deleted)PersonalPresets.Delete(Folder,id,kind);}
   }catch(Exception e){LogDesktop(e);MessageBox.Show(simpleSettings,e.Message,L.T("Импорт","Import"));}
  }
  }finally{presetEditing=false;}
 }
 async void QueueUpdateCheck(){await CheckDesktopUpdates(false);}
 void MaybeOfferDesktopUpdate(){
  if(availableRelease==null||!settings.AutomaticUpdates||settings.LastNotifiedUpdate==availableRelease.Version.ToString()||updateInstalling||presetEditing||messagePending||settings.Paused||FullscreenDetector.IsFullscreen()||(cycle!=null&&!cycle.Finished))return;
  settings.LastNotifiedUpdate=availableRelease.Version.ToString();Save();if(petMessage!=null)petMessage.Close();
  var release=availableRelease;var notice=new PetMessage(L.T("Доступна новая версия Кавычки: ","A new Kavychka version is available: ")+release.Version+". "+L.T("Модели и настройки сохранятся.","Models and settings will be preserved."),L.T("Обновить","Update"),L.T("Позже","Later"));
  notice.Confirmed=async delegate{notice.Close();await InstallDesktopUpdate(release);};notice.StopVoice=delegate{notice.Hide();};petMessage=notice;if(pet!=null)notice.AnchorTo(pet);notice.Show();
 }
 async Task CheckDesktopUpdates(bool interactive){
  if(updateChecking||updateInstalling||TestMode)return;updateChecking=true;nextUpdateCheckUtc=DateTime.UtcNow.AddHours(6);
  try{
   availableRelease=await AppUpdates.Check(CancellationToken.None);
   if(!interactive)return;
   if(availableRelease==null){MessageBox.Show(L.T("Новых доступных публичных сборок не найдено.","No newer public releases are available."),"Kavychka");return;}
   var release=availableRelease;
   if(MessageBox.Show(L.T("Доступна версия ","Version available: ")+release.Version+"\n"+release.Name+"\n\n"+L.T("Обновить приложение? Модели и пользовательские настройки сохранятся.","Update the app? Models and user settings will be preserved."),"Kavychka",MessageBoxButtons.YesNo,MessageBoxIcon.Information)!=DialogResult.Yes)return;
   await InstallDesktopUpdate(release);
  }catch(Exception error){LogDesktop(error);if(interactive)MessageBox.Show(L.T("Не удалось проверить обновления. Подробности в logs/desktop.log.","Could not check for updates. See logs/desktop.log."));}
  finally{updateChecking=false;}
 }
 async Task InstallDesktopUpdate(AppRelease release){
  if(updateInstalling)return;
  try{
   updateInstalling=true;
   updateCancel=new CancellationTokenSource();updatePercent=0;
   string installer=await AppUpdates.Download(release,Folder,updateCancel.Token,(done,total)=>{updatePercent=(int)(done*100/total);preparationStatus=L.T("Обновление: ","Update: ")+updatePercent+"%";});
   while(activeJobs.Count>0||preparationBusy||modelSetupBusy||runtimeInstalling||storageMoving||presetEditing||nativeTasks>0){preparationStatus=L.T("Обновление готово. Жду окончания текущей операции.","Update ready. Waiting for the current operation.");await Task.Delay(500,updateCancel.Token);if(exiting)return;}
   updateCancel.Token.ThrowIfCancellationRequested();Save();
   var info=new System.Diagnostics.ProcessStartInfo(installer,"--update --target "+MediaTools.Q(Application.StartupPath)+" --wait-pid "+System.Diagnostics.Process.GetCurrentProcess().Id+" --data-dir "+MediaTools.Q(Folder)+" --language "+settings.UiLanguage){UseShellExecute=false,CreateNoWindow=true};
   // Launch first: if Windows rejects the installer, the current backend/timers
   // must remain usable. The installer waits for this process before copying.
   System.Diagnostics.Process.Start(info);exiting=true;Close();
  }catch(OperationCanceledException){preparationStatus=L.T("Обновление отменено. Можно продолжать пользоваться приложением.","Update cancelled. You can keep using the app.");}
  catch(Exception error){LogDesktop(error);MessageBox.Show(L.T("Обновление не началось. Текущая версия сохранена. Подробности в logs/desktop.log.","Update did not start. The current version is unchanged. See logs/desktop.log."));}
  finally{updateInstalling=false;if(updateCancel!=null){updateCancel.Dispose();updateCancel=null;}}
 }
}
