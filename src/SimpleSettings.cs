using System;
using System.Collections.Generic;
using System.Drawing;
using System.IO;
using System.Linq;
using System.Web.Script.Serialization;
using System.Windows.Forms;

public sealed class SimpleSettingsForm:NightForm {
 readonly Settings draft;
 readonly Settings source;
 readonly JavaScriptSerializer json=new JavaScriptSerializer();
 readonly FlowLayoutPanel content;
 readonly Dictionary<string,Control> fields=new Dictionary<string,Control>();
 readonly List<Dictionary<string,object>> presets;
 readonly StudioCard overview;
 readonly Label modelStatus,storagePath,hardwareInfo;
 Dictionary<string,object> hardwareSnapshot,deviceSnapshot;
 readonly ProgressBar modelProgress;
 readonly StudioButton prepareButton,cancelButton;
 readonly StudioButton updatesButton;
 string baselineTextChoice,baselineVoiceChoice,baselineCharacterChoice;
 bool baselineRuntime,baselineSwear;
 bool changingTheme;
 readonly System.Windows.Forms.Timer refreshTimer=new System.Windows.Forms.Timer{Interval=1000};
 public Action<Settings> Saved;
 public Action Prepare,Advanced,Updates,CancelPreparation,ChangeStorage,Breakfast,Stretch,DayLibrary;
 public Action<SimpleSettingsForm> RefreshStatus;
 public Action<string> Import;
 public Action<string,string> ManagePreset;
 public Action<string> ThemeChanged;
 public string PreparationStatus="";
 public SimpleSettingsForm(Settings current,List<Dictionary<string,object>> available,bool first) {
  source=current;draft=json.Deserialize<Settings>(json.Serialize(current));presets=available;StudioColors.SetMode(current.ThemeMode);
  Text=L.T("Кавычка · твоё пространство","Kavychka · your space");ClientSize=new Size(860,760);MinimumSize=new Size(800,650);StartPosition=FormStartPosition.CenterScreen;BackColor=StudioColors.Background;ForeColor=StudioColors.Ink;Font=new Font("Segoe UI",10);AutoScaleMode=AutoScaleMode.Dpi;
  var tabs=new StudioPages{Dock=DockStyle.Fill};Controls.Add(tabs);
  content=Page(tabs,L.T("Обзор","Overview"));
  Title(first?L.T("Привет, я Кавычка","Hello, I am Kavychka"):L.T("Всё в своём темпе","At your own pace"));
  TextLine(L.T("Напоминания уже работают без загрузки моделей. Голос, характер и внешность можно менять независимо.","Reminders work without model downloads. Voice, character and appearance can be changed independently."));
  overview=new StudioCard(L.T("Ближайшее напоминание","Next reminder"),L.T("По твоему расписанию","Following your schedule"));content.Controls.Add(overview);
  var quick=new FlowLayoutPanel{Width=535,Height=53,Margin=new Padding(0,0,0,12)};content.Controls.Add(quick);
  var breakfast=new StudioButton{Text=L.T("Я позавтракала / позавтракал","I have had breakfast"),Width=290,Height=42};quick.Controls.Add(breakfast);breakfast.Click+=delegate{if(Breakfast!=null)Breakfast();};
  var stretch=new StudioButton{Text=L.T("Размяться сейчас","Stretch now"),Primary=false,Width=220,Height=42};quick.Controls.Add(stretch);stretch.Click+=delegate{if(Stretch!=null)Stretch();};
  ActionButton(L.T("Фразы на сегодня · просмотр и отбор","Today's phrases · preview and review"),delegate{if(SaveDraft(false)&&DayLibrary!=null)DayLibrary();}).Primary=false;
  Toggle("RunAtLogin",L.T("Запускать вместе с Windows","Start when I sign in to Windows"),draft.RunAtLogin);
  TextLine(L.T("После входа появится маскот. Напоминания и подготовка дня работают, пока Кавычка запущена.","The mascot appears after sign-in. Reminders and day preparation work while Kavychka is running."));
  Choice("ThemeMode",L.T("Тема","Theme"),new[]{"system","light","dark"},new[]{L.T("Как в системе","System"),L.T("Светлая","Light"),L.T("Тёмная","Dark")},draft.ThemeMode);
  ((ComboBox)fields["ThemeMode"]).SelectedIndexChanged+=delegate{if(changingTheme)return;string mode=ChoiceId("ThemeMode"),previous=draft.ThemeMode;try{if(ThemeChanged!=null)ThemeChanged(mode);draft.ThemeMode=mode;StudioColors.SetMode(mode);}catch(Exception error){changingTheme=true;try{SelectChoice("ThemeMode",previous);StudioColors.SetMode(previous);}finally{changingTheme=false;}MessageBox.Show(this,error.Message,L.T("Не удалось сохранить оформление","Could not save appearance"));}};
  Choice("UiLanguage",L.T("Язык интерфейса","Interface language"),new[]{"ru","en"},new[]{"Русский","English"},draft.UiLanguage);
  Choice("SpeechLanguage",L.T("Язык напоминаний","Reminder language"),new[]{"ru","en"},new[]{"Русский","English"},draft.SpeechLanguage);
  TextLine(L.T("Язык интерфейса изменится при следующем открытии настроек.","The interface language changes when you next open settings."));
  Toggle("AutomaticUpdates",L.T("Автоматически проверять обновления","Check for updates automatically"),draft.AutomaticUpdates);
  updatesButton=tabs.AddFooter(L.T("Обновления","Updates"),0,delegate{if(SaveDraft(false)&&Updates!=null)Updates();});
  tabs.AddFooter(L.T("Расширенные","Advanced"),1,delegate{if(SaveDraft(false)&&Advanced!=null)Advanced();});
  content=Page(tabs,L.T("Расписание","Schedule"));Title(L.T("Ритм дня","Daily rhythm"));
  TextLine(L.T("Выбери дни и время для перерывов и еды. Разминка после завтрака работает отдельно, включая выходные.","Choose reminder days and hours for breaks and meals. The stretch timer works separately, including weekends."));
  Label(L.T("Дни напоминаний","Reminder days"));var days=new StudioDays(draft.WorkDays);fields["Days"]=days;content.Controls.Add(days);
  Clocks(new[]{"WorkStart","WorkEnd"},new[]{L.T("Начало рабочего дня","Workday starts"),L.T("Окончание рабочего дня","Workday ends")},new[]{draft.WorkStart,draft.WorkEnd});Number("BreakEvery",L.T("Перерыв каждые N минут","Break every N minutes"),draft.BreakEvery,5,720);
  Toggle("MealsEnabled",L.T("Напоминания о еде","Meal reminders"),draft.MealsEnabled);
  Clocks(new[]{"BreakfastTime","LunchTime","DinnerTime"},new[]{L.T("Завтрак","Breakfast"),L.T("Обед","Lunch"),L.T("Ужин","Dinner")},new[]{draft.BreakfastTime,draft.LunchTime,draft.DinnerTime});
  Toggle("ExerciseEnabled",L.T("Разминка после отметки завтрака — каждый день","Stretch after marking breakfast — every day"),draft.ExerciseEnabled);Number("ExerciseAfter",L.T("Через сколько минут после завтрака","Minutes after breakfast"),draft.ExerciseAfter,1,1440);
  var zones=TimeZoneInfo.GetSystemTimeZones();Choice("TimeZoneId",L.T("Часовой пояс","Time zone"),new[]{""}.Concat(zones.Select(z=>z.Id)).ToArray(),new[]{L.T("Как в Windows","Follow Windows")}.Concat(zones.Select(z=>z.DisplayName)).ToArray(),draft.TimeZoneId);
  content=Page(tabs,L.T("Напоминания","Reminders"));Title(L.T("Напоминать, не мешать","A nudge, not a nuisance"));
  Toggle("Voice",L.T("Озвучивать сообщения","Read messages aloud"),draft.Voice);Toggle("NotificationSound",L.T("Сигнал перед озвучкой","Chime before speech"),draft.NotificationSound);Toggle("Swear",L.T("Разрешить мат в дразнящих фразах","Allow profanity in teasing reminders"),draft.Swear);
  Number("IdleMinutes",L.T("Неактивность, минут","Inactivity, minutes"),draft.IdleMinutes,1,120);Number("AllowedClicks",L.T("Допустимые клики за весь период","Allowed clicks during the whole period"),draft.AllowedClicks,0,100);
  TextLine(L.T("Короткие движения мыши допускаются. Прокрутка, набор текста и продолжительное движение считаются активностью. Содержимое ввода не записывается.","Small mouse movements are allowed. Scrolling, typing and sustained movement count as activity. Input content is never recorded."));
  Toggle("TrustConfirmation",L.T("Доверять кнопке «Я встала / встал»","Trust the 'I am getting up' button"),draft.TrustConfirmation);
  Number("RepeatMinutes",L.T("Интервал повторов, минут","Repeat interval, minutes"),draft.RepeatMinutes,1,120);Number("BreakRepeats",L.T("Повторов для перерыва","Break follow-ups"),draft.BreakRepeats,0,20);Number("ExerciseRepeats",L.T("Повторов для разминки","Stretch follow-ups"),draft.ExerciseRepeats,0,20);Number("MealRepeats",L.T("Повторов для еды","Meal follow-ups"),draft.MealRepeats,0,20);
  Choice("FullscreenVisual",L.T("Поверх полноэкранных приложений","Over fullscreen apps"),new[]{"both","message","none"},new[]{L.T("Маскот и сообщение","Mascot and message"),L.T("Только сообщение","Message only"),L.T("Ничего","Nothing")},draft.FullscreenVisual);
  Choice("FullscreenAudio",L.T("Звук в полноэкранных приложениях","Fullscreen audio"),new[]{"all","chime","voice","silent"},new[]{L.T("Сигнал и голос","Chime and voice"),L.T("Только сигнал","Chime only"),L.T("Только голос","Voice only"),L.T("Тишина","Silent")},draft.FullscreenAudio);
  Toggle("DeferGenerationFullscreen",L.T("Откладывать тяжёлую генерацию в полноэкранном режиме","Defer heavy generation during fullscreen apps"),draft.DeferGenerationFullscreen);
  content=Page(tabs,L.T("Персонаж","Character"));Title(L.T("Сочетай как угодно","Mix and match"));
  Entry("UserName",L.T("Как тебя называть (необязательно)","Your name (optional)"),draft.UserName);
  Choice("AddressForm",L.T("Обращение","Address form"),new[]{"neutral","female","male"},new[]{L.T("Нейтральное","Neutral"),L.T("Женский род","Feminine"),L.T("Мужской род","Masculine")},draft.AddressForm);
  PresetChoice("VoiceProfile","voice",L.T("Голосовой референс","Voice reference"),draft.VoiceProfile);
  PresetToolbar("VoiceProfile","voice");
  PresetChoice("CharacterFile","character",L.T("Характер","Personality"),draft.CharacterFile);
  PresetToolbar("CharacterFile","character");
  PresetChoice("VisualFile","visual",L.T("Внешность","Appearance"),draft.VisualFile);
  PresetToolbar("VisualFile","visual");
  Number("PetSize",L.T("Размер маскота","Mascot size"),draft.PetSize,80,260);
  Toggle("PetVisible",L.T("Показывать маскота на рабочем столе","Show the desktop mascot"),draft.PetVisible);
  content=Page(tabs,L.T("Модели","Models"));Title(L.T("Улучшенные фразы и голос","Enhanced text and speech"));
  TextLine(L.T("Это необязательно. Базовые напоминания работают сразу. Выбери два пункта — компоненты подготовятся автоматически. Большие модели требуют места и времени на первую загрузку.","This is optional. Basic reminders work immediately. Choose two options and components will be prepared automatically. Large models need disk space and time for the first download."));
  Choice("TextChoice",L.T("Текстовая модель","Text model"),new[]{"system","text-gemma4-12b","text-gemma4-e4b","text-qwen35-4b","text-qwen35-9b","text-ministral3-8b","text-qvikhr3-8b","text-saiga-llama3-8b"},new[]{L.T("Готовые фразы · без загрузки","Ready-made phrases · no download"),"Gemma 4 12B · 6.98 GB","Gemma 4 E4B · 5.15 GB","Qwen3.5 4B · 3.01 GB","Qwen3.5 9B · 6.17 GB","Ministral 3 8B · 5.20 GB","QVikhr 3 8B · 5.03 GB","Saiga Llama3 8B · 4.92 GB"},draft.TextGeneration?(draft.RuntimeGeneration?draft.RuntimeTextModel:draft.TextModel):"system");
  Choice("VoiceChoice",L.T("Голосовая модель","Speech model"),new[]{"system","tts-cosyvoice3","tts-0.6b","tts-1.7b"},new[]{L.T("Голос Windows · без загрузки","Windows voice · no download"),"CosyVoice 3 · 5.43 GB","Qwen3-TTS 0.6B · 2.52 GB","Qwen3-TTS 1.7B · 4.54 GB"},draft.VoiceProvider=="System"?"system":draft.VoiceModel);
  hardwareInfo=new Label{Text=L.T("Ускорение и память будут проверены перед загрузкой выбранных моделей.","Acceleration and memory will be checked before downloading selected models."),Width=525,Height=58,ForeColor=StudioColors.Muted,Margin=new Padding(0,12,0,0)};content.Controls.Add(hardwareInfo);
  ((ComboBox)fields["TextChoice"]).SelectedIndexChanged+=delegate{UpdateHardware(hardwareSnapshot,deviceSnapshot);};
  ((ComboBox)fields["VoiceChoice"]).SelectedIndexChanged+=delegate{UpdateHardware(hardwareSnapshot,deviceSnapshot);};
  Toggle("RuntimeGeneration",L.T("Рантайм — новая фраза при каждом напоминании","Runtime — generate a fresh phrase for each reminder"),draft.RuntimeGeneration);
  TextLine(L.T("Без рантайма фразы готовятся заранее на день. Если новая запись не успеет, прозвучит готовая или системный голос; начатая генерация сохранится.","Without runtime, phrases are prepared ahead for the day. If a new recording is not ready in time, a prepared recording or system speech is used; generation is kept."));
  prepareButton=ActionButton(L.T("Подготовить выбранное","Prepare selected models"),delegate{if(SaveDraft(false)&&Prepare!=null)Prepare();});
  modelStatus=new Label{Text=L.T("Готовые модели повторно не скачиваются.","Installed models are reused."),Width=525,Height=68,ForeColor=StudioColors.Muted,Margin=new Padding(0,10,0,0)};content.Controls.Add(modelStatus);
  modelProgress=new ProgressBar{Width=525,Height=7,Visible=false,Margin=new Padding(0,0,0,10)};content.Controls.Add(modelProgress);
  cancelButton=ActionButton(L.T("Отменить подготовку","Cancel preparation"),delegate{if(CancelPreparation!=null)CancelPreparation();});cancelButton.Primary=false;cancelButton.Visible=false;
  Label(L.T("Общее хранилище","Shared storage"));storagePath=new Label{Text=L.T("Модели, записи и настройки сохраняются между обновлениями.","Models, recordings and settings survive app updates."),Width=525,Height=66,ForeColor=StudioColors.Muted};content.Controls.Add(storagePath);
  ActionButton(L.T("Выбрать папку хранилища","Choose storage folder"),delegate{if(SaveDraft(false)&&ChangeStorage!=null)ChangeStorage();}).Primary=false;
  var modelActions=new Panel{Height=146,BackColor=StudioColors.Background};
  modelActions.Controls.Add(prepareButton);prepareButton.SetBounds(32,12,310,42);
  modelActions.Controls.Add(cancelButton);cancelButton.SetBounds(352,12,205,42);
  modelActions.Controls.Add(modelStatus);modelStatus.SetBounds(32,65,525,59);
  modelActions.Controls.Add(modelProgress);modelProgress.SetBounds(32,132,525,7);
  tabs.PinFooter(content,modelActions);
  var bottom=new Panel{Dock=DockStyle.Bottom,Height=72,BackColor=StudioColors.Background};Controls.Add(bottom);
  bottom.Controls.Add(new Label{Text=L.T("Настройки хранятся только на твоём компьютере","Settings stay on your computer"),Location=new Point(227,25),Size=new Size(360,32),Font=new Font("Segoe UI",8),ForeColor=StudioColors.Muted});
  var save=new StudioButton{Text=first?L.T("Начать пользоваться","Let's begin"):L.T("Сохранить","Save changes"),Width=200,Height=42,Location=new Point(ClientSize.Width-228,12),Anchor=AnchorStyles.Right|AnchorStyles.Top};bottom.Controls.Add(save);save.Click+=delegate{if(SaveDraft(true))Close();};
  refreshTimer.Tick+=delegate{if(RefreshStatus!=null)RefreshStatus(this);};refreshTimer.Start();
  baselineTextChoice=ChoiceId("TextChoice");baselineVoiceChoice=ChoiceId("VoiceChoice");baselineCharacterChoice=ChoiceId("CharacterFile");baselineRuntime=draft.RuntimeGeneration;baselineSwear=draft.Swear;
 }
 string ChoiceId(string key){return ((Item)((ComboBox)fields[key]).SelectedItem).Id;}
 void SelectChoice(string key,string id){var combo=(ComboBox)fields[key];for(int i=0;i<combo.Items.Count;i++)if(((Item)combo.Items[i]).Id==id){combo.SelectedIndex=i;return;}}
 public void RebaseModelChoices(Settings live){
  var runtime=(CheckBox)fields["RuntimeGeneration"];bool cleanRuntime=runtime.Checked==baselineRuntime;
  bool cleanText=ChoiceId("TextChoice")==baselineTextChoice&&cleanRuntime;
  if(cleanRuntime){runtime.Checked=live.RuntimeGeneration;baselineRuntime=live.RuntimeGeneration;}
  if(cleanText){SelectChoice("TextChoice",live.TextGeneration?(live.RuntimeGeneration?live.RuntimeTextModel:live.TextModel):"system");baselineTextChoice=ChoiceId("TextChoice");}
  if(ChoiceId("VoiceChoice")==baselineVoiceChoice){SelectChoice("VoiceChoice",live.VoiceProvider=="System"?"system":live.VoiceModel);baselineVoiceChoice=ChoiceId("VoiceChoice");}
  var swear=(CheckBox)fields["Swear"];if(swear.Checked==baselineSwear){swear.Checked=live.Swear;baselineSwear=live.Swear;}
  // The inactive model has no visible selector. Keep its migrated value while
  // leaving any active, unsaved choice in the controls for SaveDraft to apply.
  draft.TextModel=live.TextModel;draft.RuntimeTextModel=live.RuntimeTextModel;draft.VoiceModel=live.VoiceModel;
 }
 public void RebaseCharacterChoice(Settings live,List<Dictionary<string,object>> available){if(ChoiceId("CharacterFile")!=baselineCharacterChoice)return;ReloadPresets(available,"character",live.CharacterFile);baselineCharacterChoice=ChoiceId("CharacterFile");}
 public void UpdateStatus(string next,string detail,bool busy,int percent,bool indeterminate,string root){overview.Detail.Text=next;modelStatus.Text=detail;prepareButton.Enabled=!busy;cancelButton.Visible=busy;modelProgress.Visible=busy;modelProgress.Style=indeterminate?ProgressBarStyle.Marquee:ProgressBarStyle.Blocks;modelProgress.Value=Math.Max(0,Math.Min(100,percent));storagePath.Text=root+Environment.NewLine+L.T("Одна папка для всех версий приложения","One folder shared by all app versions");}
 public void UpdateVersion(string version,bool busy){updatesButton.Text=string.IsNullOrEmpty(version)?L.T("Обновления","Updates"):L.T("Обновить · ","Update · ")+version;updatesButton.Enabled=!busy;}
 public void UpdateHardware(Dictionary<string,object> hardware,Dictionary<string,object> devices){
  hardwareSnapshot=hardware;deviceSnapshot=devices;if(hardware==null)return;
  Func<string,double> value=key=>hardware.ContainsKey(key)?Convert.ToDouble(hardware[key]):0;
  bool textGpu=devices!=null&&devices.ContainsKey("text_gpu")&&Convert.ToBoolean(devices["text_gpu"]),voiceGpu=devices!=null&&devices.ContainsKey("speech_gpu")&&Convert.ToBoolean(devices["speech_gpu"]);
  double ram=value("ram_bytes")/1e9,vram=value("vram_bytes")/1e9;string name=hardware.ContainsKey("gpu_name")?Convert.ToString(hardware["gpu_name"]):"";
  string detail=(ram>0?"RAM "+ram.ToString("0.0")+" GB · ":"")+L.T("Текст: ","Text: ")+(textGpu?"NVIDIA":"CPU")+" · "+L.T("Голос: ","Speech: ")+(voiceGpu?"NVIDIA":"CPU");
  if(name.Length>0)detail+=Environment.NewLine+name+" · "+vram.ToString("0.0")+" GB VRAM";
  int textIndex=((ComboBox)fields["TextChoice"]).SelectedIndex;
  double weights=new[]{0,6.98,5.15,3.01,6.17,5.20,5.03,4.92}[Math.Max(0,textIndex)];
  if(weights>0&&textGpu&&vram>0&&vram<weights*1.2+1)detail+=Environment.NewLine+L.T("Видеопамяти для этого текста может не хватить. Выбери модель меньше или CPU в расширенных настройках.","This text model may exceed VRAM. Choose a smaller model or CPU in Advanced.");
  else if(weights>0&&!textGpu&&ram>0&&ram<weights*1.2+2)detail+=Environment.NewLine+L.T("Для этого текста может не хватить RAM. Выбери модель меньше.","This text model may exceed RAM. Choose a smaller model.");
  if(!voiceGpu&&ChoiceId("VoiceChoice")!="system")detail+=Environment.NewLine+L.T("Подготовка локальной озвучки на CPU может занимать много времени.","Preparing local speech on CPU can take a long time.");
  if(hardwareInfo.Text!=detail){hardwareInfo.Text=detail;hardwareInfo.Height=TextRenderer.MeasureText(detail,hardwareInfo.Font,new Size(525,int.MaxValue),TextFormatFlags.WordBreak).Height+18;}
 }
 public void ReloadPresets(List<Dictionary<string,object>> available,string kind,string id){presets.Clear();presets.AddRange(available);string key=kind=="voice"?"VoiceProfile":kind=="character"?"CharacterFile":"VisualFile";var combo=(ComboBox)fields[key];combo.Items.Clear();combo.Items.Add(new Item{Id="",Name=L.T("Стандартная Кавычка","Default Kavychka")});foreach(var item in presets.Where(p=>Convert.ToString(p["kind"])==kind))combo.Items.Add(new Item{Id=Convert.ToString(item["id"]),Name=Convert.ToString(item["name"])});combo.SelectedIndex=0;for(int i=0;i<combo.Items.Count;i++)if(((Item)combo.Items[i]).Id==id)combo.SelectedIndex=i;}
 public void SelectPage(int index){foreach(Control c in Controls)if(c is StudioPages)((StudioPages)c).Select(index);}
 protected override void Dispose(bool disposing){if(disposing)refreshTimer.Dispose();base.Dispose(disposing);}
 protected override void OnLoad(EventArgs e){base.OnLoad(e);foreach(Control child in Controls)if(child is StudioPages)((StudioPages)child).InitializeScrolling();StudioColors.NativeTheme(this);}
 FlowLayoutPanel Page(StudioPages tabs,string name){return tabs.AddPage(name);}
 void Title(string text){content.Controls.Add(new Label{Text=text,Width=535,Height=52,Font=new Font("Segoe UI",22,FontStyle.Bold),ForeColor=ForeColor});}
 void TextLine(string text){int height=TextRenderer.MeasureText(text,new Font("Segoe UI",10),new Size(535,int.MaxValue),TextFormatFlags.WordBreak).Height+14;content.Controls.Add(new Label{Text=text,Width=535,Height=height,ForeColor=StudioColors.Muted,AutoSize=false,Font=new Font("Segoe UI",10)});}
 void Label(string text){content.Controls.Add(new Label{Text=text,Width=535,Height=28,ForeColor=ForeColor,Margin=new Padding(0,14,0,0)});}
 void Entry(string key,string label,string value){Label(label);var c=new TextBox{Text=value,Width=525,MaxLength=key=="UserName"?80:160,BackColor=StudioColors.Surface,ForeColor=ForeColor,Font=new Font("Segoe UI",12),BorderStyle=BorderStyle.FixedSingle};fields[key]=c;content.Controls.Add(c);}
 void Clocks(string[] keys,string[] labels,int[] minutes){var row=new FlowLayoutPanel{Width=535,Height=82,WrapContents=false,Margin=new Padding(0,8,0,8)};int width=keys.Length==2?260:170;for(int i=0;i<keys.Length;i++){var cell=new Panel{Width=width,Height=78,Margin=new Padding(0,0,5,0)};cell.Controls.Add(new Label{Text=labels[i],Location=new Point(3,0),Size=new Size(width-6,29),ForeColor=ForeColor});var c=new StudioClock(minutes[i]){Width=width-15,Location=new Point(3,34)};fields[keys[i]]=c;cell.Controls.Add(c);row.Controls.Add(cell);}content.Controls.Add(row);}
 void Number(string key,string label,int value,int min,int max){Label(label);var c=new NumericUpDown{Minimum=min,Maximum=max,Value=Math.Max(min,Math.Min(max,value)),Width=180,BackColor=StudioColors.Surface,ForeColor=ForeColor,Font=new Font("Segoe UI",12)};fields[key]=c;content.Controls.Add(c);}
 void Toggle(string key,string label,bool value){var c=new StudioToggle{Text=label,Checked=value,Width=535,Height=48,ForeColor=ForeColor};fields[key]=c;content.Controls.Add(c);}
 sealed class Item{public string Id,Name;public override string ToString(){return Name;}}
 void Choice(string key,string label,string[] ids,string[] names,string value){Label(label);var c=new StudioCombo{DropDownStyle=ComboBoxStyle.DropDownList,DrawMode=DrawMode.OwnerDrawFixed,ItemHeight=30,Width=525,BackColor=StudioColors.Surface,ForeColor=ForeColor};c.DrawItem+=delegate(object sender,DrawItemEventArgs e){bool selected=(e.State&DrawItemState.Selected)!=0;using(var brush=new SolidBrush(selected?StudioColors.Soft:c.BackColor))e.Graphics.FillRectangle(brush,e.Bounds);if(e.Index>=0)TextRenderer.DrawText(e.Graphics,c.GetItemText(c.Items[e.Index]),c.Font,new Rectangle(e.Bounds.X+8,e.Bounds.Y,e.Bounds.Width-10,e.Bounds.Height),c.ForeColor,TextFormatFlags.Left|TextFormatFlags.VerticalCenter);e.DrawFocusRectangle();};for(int i=0;i<ids.Length;i++)c.Items.Add(new Item{Id=ids[i],Name=names[i]});c.SelectedIndex=Math.Max(0,Array.IndexOf(ids,value));fields[key]=c;content.Controls.Add(c);}
 void PresetChoice(string key,string kind,string label,string value){var choices=presets.Where(p=>Convert.ToString(p["kind"])==kind).ToArray();Choice(key,label,new[]{""}.Concat(choices.Select(p=>Convert.ToString(p["id"]))).ToArray(),new[]{L.T("Стандартная Кавычка","Default Kavychka")}.Concat(choices.Select(p=>Convert.ToString(p["name"]))).ToArray(),value);}
 void PresetToolbar(string key,string kind){var row=new FlowLayoutPanel{Width=535,Height=53,Margin=new Padding(0,8,0,0)};content.Controls.Add(row);var edit=new StudioButton{Text=L.T("Просмотр и настройка","Preview and settings"),Primary=false,Width=255,Height=40};row.Controls.Add(edit);edit.Click+=delegate{if(ManagePreset!=null)ManagePreset(kind,((Item)((ComboBox)fields[key]).SelectedItem).Id);};var import=new StudioButton{Text=L.T("Загрузить файл / пресет","Import file / preset"),Primary=false,Width=255,Height=40};row.Controls.Add(import);import.Click+=delegate{if(Import!=null)Import(kind);};}
 StudioButton ActionButton(string text,Action action){var b=new StudioButton{Text=text,Width=525,Height=42,Margin=new Padding(0,12,0,8)};content.Controls.Add(b);b.Click+=delegate{action();};return b;}
 bool SaveDraft(bool finish){try{
  draft.AutomaticPreparation=source.AutomaticPreparation;draft.AutoKeepDaily=source.AutoKeepDaily;
  foreach(var pair in fields){var field=typeof(Settings).GetField(pair.Key);if(field==null)continue;object value=null;
   if(pair.Value is StudioClock)value=((StudioClock)pair.Value).Minutes;else if(pair.Value is NumericUpDown)value=(int)((NumericUpDown)pair.Value).Value;else if(pair.Value is CheckBox)value=((CheckBox)pair.Value).Checked;else if(pair.Value is ComboBox)value=((Item)((ComboBox)pair.Value).SelectedItem).Id;else value=pair.Value.Text.Trim();field.SetValue(draft,value);
  }
  draft.WorkDays=((StudioDays)fields["Days"]).Days;
  string textId=((Item)((ComboBox)fields["TextChoice"]).SelectedItem).Id,voiceId=((Item)((ComboBox)fields["VoiceChoice"]).SelectedItem).Id;
  draft.TextGeneration=textId!="system";if(draft.TextGeneration){if(draft.RuntimeGeneration)draft.RuntimeTextModel=textId;else draft.TextModel=textId;}draft.VoiceProvider=voiceId=="system"?"System":"Qwen local";if(voiceId!="system")draft.VoiceModel=voiceId;
  RitualSchedule.Validate(draft);if(finish)draft.SetupComplete=true;if(Saved!=null)Saved(draft);return true;
 }catch(Exception error){MessageBox.Show(this,error.Message,L.T("Проверь настройки","Check settings"));return false;}}
}
