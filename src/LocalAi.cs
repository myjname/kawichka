using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.IO.Compression;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Security.Cryptography;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Windows.Forms;

public partial class Settings { public bool LocalAiMigrated = false; }

public partial class MainForm {
 static string ResolveFolder() {
  return SharedStorage.Resolve(Application.StartupPath,Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),Environment.GetCommandLineArgs());
 }
 void MigrateSettings() {
  if(!SharedStorage.IsIsolated(Environment.GetCommandLineArgs()))SharedStorage.Preferences(Folder,SharedStorage.Pending(Application.StartupPath,Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),Folder));
 }
 Process backend;
 Task backendStarting;
 Task storagePreparing;
 bool storagePrepared;
 string[] pendingStorage=new string[0];
 HttpClient localHttp;
 string endpoint;
 CancellationTokenSource bootstrapCancel;
 CheckBox runtimeToggle,autoDaily,autoKeep,localSwear;
 NumericUpDown phraseCount;
 ComboBox ttsChoice,textChoice,runtimeTextChoice,deviceChoice,deliveryChoice;
 CheckedListBox categoryChoice;
 ListView library;
 TextBox selectedText,characterDescription,deliveryInstruction;
 string defaultCharacter;
 Label localProgress,libraryInfo,modelInfo,textModelInfo,runtimeTextModelInfo,storageInfo;
 ProgressBar localBar;
 readonly HashSet<string> activeJobs=new HashSet<string>();
 readonly Dictionary<string,Label> componentLabels=new Dictionary<string,Label>();
 readonly Dictionary<string,string> componentStates=new Dictionary<string,string>();
 static readonly string[] TextModelIds={"text-gemma4-12b","text-gemma4-e4b","text-qwen35-4b","text-qwen35-9b","text-ministral3-8b","text-qvikhr3-8b","text-saiga-llama3-8b"};
 static object[] TextModelNames {get{return new object[]{L.T("Gemma 4 12B · Q4_0 · 6,98 ГБ","Gemma 4 12B · Q4_0 · 6.98 GB"),L.T("Gemma 4 E4B · Q4_0 · 5,15 ГБ","Gemma 4 E4B · Q4_0 · 5.15 GB"),L.T("Qwen3.5 4B · Q4_K_M · 3,01 ГБ","Qwen3.5 4B · Q4_K_M · 3.01 GB"),L.T("Qwen3.5 9B · Q4_K_M · 6,17 ГБ","Qwen3.5 9B · Q4_K_M · 6.17 GB"),L.T("Ministral 3 8B · Q4_K_M · 5,20 ГБ","Ministral 3 8B · Q4_K_M · 5.20 GB"),L.T("QVikhr 3 8B · Q4_K_M · 5,03 ГБ","QVikhr 3 8B · Q4_K_M · 5.03 GB"),L.T("Saiga-Llama3 8B · Q4_K · 4,92 ГБ · не проверялась","Saiga-Llama3 8B · Q4_K · 4.92 GB · not tested")};}}
 static readonly string[] VoiceModelIds={"tts-cosyvoice3","tts-0.6b","tts-1.7b"};
 string[] DeliveryIds={"auto","confident","calm","amused"};
 object[] DeliveryNames;
 readonly List<Dictionary<string,object>> deliveryCatalog=new List<Dictionary<string,object>>();
 TextBox deliveryName;
 bool deliveryEditing;
 void PopulateDeliveries(string selected) {
  deliveryEditing=true;
  DeliveryIds=new[]{"auto"}.Concat(deliveryCatalog.Select(item=>S(item,"id"))).ToArray();
  DeliveryNames=new object[]{L.T("Авто · подбирать к каждой новой фразе","Auto · choose for each new phrase")}.Concat(deliveryCatalog.Select(item=>(object)DeliveryDisplayName(item))).ToArray();
  deliveryChoice.Items.Clear();deliveryChoice.Items.AddRange(DeliveryNames);deliveryChoice.SelectedIndex=Math.Max(0,Array.IndexOf(DeliveryIds,selected));
  deliveryEditing=false;ShowDelivery();
 }
 void ShowDelivery() {
  if(deliveryEditing||deliveryName==null)return;
  string id=DeliveryIds[Math.Max(0,deliveryChoice.SelectedIndex)];
  var item=deliveryCatalog.FirstOrDefault(entry=>S(entry,"id")==id);
  deliveryName.Text=item==null?"":DeliveryDisplayName(item);deliveryInstruction.Text=item==null?"":S(item,"instruction");
 }
 async Task SaveDelivery(bool add) {
  string name=deliveryName.Text.Trim(),instruction=deliveryInstruction.Text.Trim();
  string id=add?"user_"+Guid.NewGuid().ToString("N"):DeliveryIds[Math.Max(0,deliveryChoice.SelectedIndex)];
  if(id=="auto")throw new Exception(L.T("Выберите подачу для изменения или нажмите «Добавить».","Choose a delivery to edit or click Add."));
  var items=deliveryCatalog.Where(item=>S(item,"id")!=id).ToList();
  items.Add(new Dictionary<string,object>{{"id",id},{"name",name},{"instruction",instruction}});
  await Api("/settings",new {delivery_catalog=items.ToArray(),cosy_style=id});
  await LoadLocalSettings();status.Text=L.T("Подача сохранена. Она доступна и для автовыбора.","Delivery saved. It is also available for automatic selection.");
 }
 async Task DeleteDelivery() {
  string id=DeliveryIds[Math.Max(0,deliveryChoice.SelectedIndex)];
  if(id=="auto")throw new Exception(L.T("Автовыбор удалить нельзя.","Automatic selection cannot be deleted."));
  var items=deliveryCatalog.Where(item=>S(item,"id")!=id).ToArray();
  await Api("/settings",new {delivery_catalog=items,cosy_style="auto"});await LoadLocalSettings();status.Text=L.T("Подача удалена из списка. Выбрано «Авто».","Delivery removed. Automatic selection is now active.");
 }
 static string SelectedTextModel(ComboBox choice) {return TextModelIds[Math.Max(0,choice.SelectedIndex)];}
 void UpdateTextModelInfo() {
  if(textModelInfo!=null)textModelInfo.Text=componentStates.ContainsKey(SelectedTextModel(textChoice))?componentStates[SelectedTextModel(textChoice)]:L.T("Установка ещё не проверена","Installation has not been checked yet");
  if(runtimeTextModelInfo!=null)runtimeTextModelInfo.Text=componentStates.ContainsKey(SelectedTextModel(runtimeTextChoice))?componentStates[SelectedTextModel(runtimeTextChoice)]:L.T("Установка ещё не проверена","Installation has not been checked yet");
 }
 System.Windows.Forms.Timer dailyTimer;
 string dailyAttempt="";
 string audioCleanupDay="";
 bool localLoading,runtimeInstalling,storageMoving;
 static readonly string[] Categories={"break","breakfast","lunch","dinner","exercise"};
 static string[] CategoryNames {get{return new string[]{L.T("Перерыв","Break"),L.T("Завтрак","Breakfast"),L.T("Обед","Lunch"),L.T("Ужин","Dinner"),L.T("Зарядка","Stretch")};}}
 static Dictionary<string,object> D(object value) { return value as Dictionary<string,object>; }
 static string S(Dictionary<string,object> obj,string key) { return obj!=null && obj.ContainsKey(key) ? Convert.ToString(obj[key]) : ""; }
 static bool B(Dictionary<string,object> obj,string key) { return obj!=null && obj.ContainsKey(key) && Convert.ToBoolean(obj[key]); }
 string Today() { return RitualSchedule.Local(settings,DateTime.UtcNow).ToString("yyyy-MM-dd"); }
 static string CharacterLines(string value) {return value.Replace("\r\n","\n").Replace("\n","\r\n");}
 ComboBox LocalCombo(Control parent,object[] values) {
  var c=new ComboBox { Width=595,DropDownStyle=ComboBoxStyle.DropDownList,BackColor=BackColor,ForeColor=ink,FlatStyle=FlatStyle.Flat };
  c.Items.AddRange(values); c.SelectedIndex=0; parent.Controls.Add(c); return c;
 }
 void InitializeLocalAi() {
  DeliveryNames=new object[]{L.T("Авто · подбирать к каждой новой фразе","Auto · choose for each new phrase"),L.T("Уверенно","Confident"),L.T("Спокойно","Calm"),L.T("С лёгкой усмешкой","Slightly amused")};
  tabs.ItemSize=new Size(132,46);
  var phrasesTab=new TabPage(L.T("Фразы на день","Daily phrases")) { BackColor=BackColor,Padding=new Padding(0,12,0,0) };
  var modelsTab=new TabPage(L.T("Модели","Models")) { BackColor=BackColor,Padding=new Padding(0,12,0,0) };
  var characterTab=new TabPage(L.T("Персонаж","Character")) { BackColor=BackColor,Padding=new Padding(0,12,0,0) };
  TabPage day=tabs.TabPages[0],options=tabs.TabPages[1];tabs.TabPages.Clear();tabs.TabPages.AddRange(new[]{day,phrasesTab,modelsTab,characterTab,options});
  var persona=D(Json.DeserializeObject(File.ReadAllText(Path.Combine(Application.StartupPath,"assets","character.json"),Encoding.UTF8)));
  defaultCharacter=CharacterLines(PersonalPresets.DefaultDescription(Application.StartupPath,L.Language));
  var cs=new NightScroll();characterTab.Controls.Add(cs);var cp=Page(cs);var cc=Card(cp);
  AddLabel(cc,L.T("Кто говорит с тобой","Who is speaking to you"),21,44).Font=new Font("Georgia",21);
  AddLabel(cc,L.T("Опишите характер, отношение к вам и манеру речи. Это описание используется при создании каждой новой фразы.","Describe the personality, attitude and manner of speaking. Each new phrase uses this description."),10,53).ForeColor=muted;
  characterDescription=new TextBox {Name="CharacterDescription",Width=595,Height=300,Multiline=true,AcceptsReturn=true,ScrollBars=ScrollBars.Vertical,MaxLength=Convert.ToInt32(persona["max_length"]),BackColor=BackColor,ForeColor=ink,BorderStyle=BorderStyle.FixedSingle,Text=defaultCharacter};cc.Controls.Add(characterDescription);
  var cr=Row(cc);
  Button(cr,L.T("Сохранить описание","Save description"),async delegate {await LocalAction(SaveCharacter);});
  Button(cr,L.T("Вернуть Кавычку","Reset to Kavychka"),delegate {characterDescription.Text=defaultCharacter;status.Text=L.T("Стандартный характер восстановлен в поле. Нажмите «Сохранить описание», чтобы применить.","The default description is back in the field. Click Save description to apply it.");});
  AddLabel(cc,L.T("По умолчанию — дружелюбная Кавычка с сухим юмором. Собственные персонажи хранятся в пользовательских пресетах. Лимит — ","The default is friendly Kavychka with dry humour. Custom characters are stored in personal presets. Limit: ")+persona["max_length"]+L.T(" символов."," characters."),9,52).ForeColor=muted;
  AddLabel(cc,L.T("После сохранения создайте «Новую партию» на вкладке «Фразы на день» или используйте рантайм. Готовый дневной список сам не заменяется.","After saving, create a New batch in Daily phrases or use runtime generation. Existing daily phrases are kept."),9,52).ForeColor=muted;
  AddLabel(cc,L.T("Поле меняет характер текста. Интонацию CosyVoice настраивайте в «Модели → Голосовые модели»: можно выбрать подачу, задать свою команду или включить автоподбор. Qwen3-TTS Base этих команд не получает.","This field controls the writing personality. Configure CosyVoice delivery in Models → Speech models. Qwen3-TTS Base does not use delivery commands."),9,68).ForeColor=muted;
  cs.SetContent(cp);
  var ps=new NightScroll(); phrasesTab.Controls.Add(ps); var pp=Page(ps);
  var planning=Card(pp);
  AddLabel(planning,L.T("Новые слова на каждый день","Fresh words for each day"),21,44).Font=new Font("Georgia",21);
  AddLabel(planning,L.T("День определяется выбранным часовым поясом. Готовые фразы звучат один раз; история сохраняется после перезапуска.","Days follow the selected time zone. Each prepared phrase plays once; history survives restarts."),10,49).ForeColor=muted;
  runtimeToggle=Check(planning,L.T("Рантайм — новая фраза при каждом уведомлении","Runtime · a new phrase for each reminder"),false);
  autoDaily=Check(planning,L.T("Готовить новый список на день автоматически","Prepare daily phrases automatically"),true);
  autoKeep=Check(planning,L.T("Сразу разрешать дневные фразы для уведомлений","Approve new daily phrases automatically"),true);
  localSwear=Check(planning,L.T("Можно мат, иронию и ласковый стёб","Allow profanity and playful teasing"),settings.Swear);
  var countRow=Row(planning); countRow.Controls.Add(new Label {Text=L.T("Фраз в партии:","Phrases per batch:"),Width=245,Height=32,TextAlign=ContentAlignment.MiddleLeft});
  phraseCount=new NumericUpDown {Minimum=1,Maximum=100,Value=13,Width=85,BackColor=BackColor,ForeColor=ink}; countRow.Controls.Add(phraseCount);
  categoryChoice=new CheckedListBox {Width=595,Height=109,CheckOnClick=true,BackColor=BackColor,ForeColor=ink,BorderStyle=BorderStyle.None};
  foreach(string name in CategoryNames) categoryChoice.Items.Add(name,true); planning.Controls.Add(categoryChoice);
  AddLabel(planning,L.T("Дневная подготовка следует расписанию. Чтобы сначала отбирать кандидатов, выключите автоматическое разрешение.","Daily preparation follows your schedule. Turn off automatic approval to review candidates first."),9,43).ForeColor=muted;
  var pr=Row(planning);
  Button(pr,L.T("Сохранить","Save"),async delegate { await LocalAction(async delegate {await SaveLocalSettings(); dailyAttempt=""; await CheckDaily();}); });
  Button(pr,L.T("Подготовить день","Prepare day"),async delegate {await LocalAction(async delegate {await SaveLocalSettings(); await RunJob("daily",new {day=Today()});});});
  Button(pr,L.T("Новая партия","New batch"),async delegate {await LocalAction(async delegate {await SaveLocalSettings(); await RunJob("batch",new {day=Today()});});});
  AddLabel(planning,L.T("Новая партия попадает в кандидаты. Выберите записи и нажмите «Оставить», чтобы разрешить их сегодня.","New batches create candidates. Select recordings and click Keep to approve them for today."),9,42).ForeColor=muted;
  var lc=Card(pp); AddLabel(lc,L.T("Библиотека записей","Recording library"),18,39).Font=new Font("Georgia",18);
  libraryInfo=AddLabel(lc,L.T("Здесь появятся дневной план и сохранённые записи.","The daily plan and saved recordings will appear here."),9,40); libraryInfo.ForeColor=muted;
  library=new ListView {Width=595,Height=255,View=View.Details,FullRowSelect=true,MultiSelect=true,HideSelection=false,BackColor=BackColor,ForeColor=ink,BorderStyle=BorderStyle.None};
  library.Columns.Add(L.T("День","Day"),80);library.Columns.Add(L.T("Категория","Category"),75);library.Columns.Add(L.T("Голос","Voice"),85);library.Columns.Add(L.T("Подача","Delivery"),90);library.Columns.Add(L.T("Состояние","State"),85);library.Columns.Add(L.T("Текст","Text"),190);lc.Controls.Add(library);
  library.OwnerDraw=true;
  library.DrawColumnHeader+=delegate(object sender,DrawListViewColumnHeaderEventArgs e){using(var brush=new SolidBrush(paper))e.Graphics.FillRectangle(brush,e.Bounds);TextRenderer.DrawText(e.Graphics,e.Header.Text,Font,e.Bounds,ink,TextFormatFlags.Left|TextFormatFlags.VerticalCenter);};
  library.DrawSubItem+=delegate(object sender,DrawListViewSubItemEventArgs e){using(var brush=new SolidBrush(e.Item.Selected?StudioColors.Soft:BackColor))e.Graphics.FillRectangle(brush,e.Bounds);TextRenderer.DrawText(e.Graphics,e.SubItem.Text,Font,e.Bounds,ink,TextFormatFlags.Left|TextFormatFlags.VerticalCenter|TextFormatFlags.EndEllipsis);};
  selectedText=new TextBox {Width=595,Height=72,Multiline=true,ReadOnly=true,ScrollBars=ScrollBars.Vertical,BackColor=BackColor,ForeColor=ink,BorderStyle=BorderStyle.None};lc.Controls.Add(selectedText);
  library.SelectedIndexChanged+=delegate { if(library.SelectedItems.Count>0) selectedText.Text=S(D(library.SelectedItems[0].Tag),"text"); };
  var lr=Row(lc);
  Button(lr,L.T("▶ Слушать","▶ Listen"),async delegate {await LocalAction(PreviewSelected);}); Button(lr,L.T("Стоп","Stop"),delegate {StopAudio();});
  Button(lr,L.T("Оставить","Keep"),async delegate {await SelectLocal("keep");}); Button(lr,L.T("Исключить","Exclude"),async delegate {await SelectLocal("exclude");});
  Button(lr,L.T("Удалить","Delete"),async delegate {await SelectLocal("delete");});
  var lr2=Row(lc);
  Button(lr2,L.T("Обновить","Refresh"),async delegate {await LocalAction(RefreshLibrary);});
  Button(lr2,L.T("Новый текст","Rewrite text"),async delegate {await LocalAction(async delegate {foreach(string id in SelectedIds()) await RunJob("rewrite",new {id=id});});});
  Button(lr2,L.T("Озвучить заново","Regenerate voice"),async delegate {await LocalAction(async delegate {foreach(string id in SelectedIds()) await RunJob("speech",new {id=id,force=true});});});
  var lr3=Row(lc);
  Button(lr3,L.T("Экспорт ZIP","Export ZIP"),async delegate {await LocalAction(ExportLocal);});
  Button(lr3,L.T("Очистить кэш","Clear cache"),async delegate {await LocalAction(async delegate {await Api("/cleanup",new {});await RefreshLibrary();});});
  var lr4=Row(lc);
  Button(lr4,L.T("Очистить библиотеку","Clear library"),async delegate {await LocalAction(ClearLibrary);});
  AddLabel(lc,L.T("ZIP: выбранные строки или вся библиотека, если ничего не выбрано. Очистка удаляет только аудио удалённых записей и временные файлы.","ZIP exports selected rows, or the whole library when none are selected. Cache cleanup removes audio of deleted rows and temporary files."),9,46).ForeColor=muted;
  ps.SetContent(pp);
  var ms=new NightScroll(); modelsTab.Controls.Add(ms);var mp=Page(ms);var mc=Card(mp);
  AddLabel(mc,L.T("Локальные модели","Local models"),21,44).Font=new Font("Georgia",21);
  AddLabel(mc,L.T("Выбор в списке ничего не скачивает. Используйте кнопку нужной модели, затем «Применить выбор». Модели текста и голоса работают последовательно.","Selecting a model does not download it. Use its download button, then Apply selection. Text and speech models run one at a time."),10,60).ForeColor=muted;
  Button(Row(mc),L.T("Применить выбор","Apply selection"),async delegate {await LocalAction(SaveLocalSettings);});
  storageInfo=AddLabel(mc,L.T("Общее хранилище для всех версий:\r\n","Shared storage for all versions:\r\n")+Folder,9,52);storageInfo.ForeColor=muted;
  Button(Row(mc),L.T("Открыть папку данных","Open data folder"),delegate {Process.Start(new ProcessStartInfo("explorer.exe","\""+Folder+"\"") {UseShellExecute=true});});
  Button(Row(mc),L.T("Изменить папку хранилища","Change storage folder"),async delegate {await LocalAction(ChangeStorage);});
  AddLabel(mc,L.T("По умолчанию — %LOCALAPPDATA%\\Kavychka. Можно выбрать пустую папку на другом диске: модели, библиотека и среда переносятся без повторного скачивания. Путь сохраняется для обновлений.","Default: %LOCALAPPDATA%\\Kavychka. Choose an empty folder on another drive to move models, the library and runtimes without downloading again. Updates use the saved path."),9,66).ForeColor=muted;
  var tc=Card(mp);AddLabel(tc,L.T("Текстовые модели","Text models"),20,42).Font=new Font("Georgia",20);
  AddLabel(tc,L.T("Создают фразы на выбранном языке напоминаний по описанию персонажа. Сравните модели и выберите подходящие формулировки.","Create phrases in the selected reminder language using the character description. You can compare models to choose the wording you prefer."),10,57).ForeColor=muted;
  AddLabel(tc,L.T("Дневной список, новые партии и замена текста:","Daily preparation, batches and rewriting:"),10,32).ForeColor=muted;
  textChoice=LocalCombo(tc,TextModelNames);
  textModelInfo=AddLabel(tc,L.T("Установка ещё не проверена","Installation has not been checked yet"),9,46);
  Button(Row(tc),L.T("Скачать выбранную текстовую","Download selected text model"),async delegate {await LocalAction(async delegate {string key=SelectedTextModel(textChoice);await EnsureBackend();await RunJob("install",new {component=key,gpu=true});});});
  AddLabel(tc,L.T("Новая фраза при каждом уведомлении (рантайм):","A new phrase for each reminder (runtime):"),10,32).ForeColor=muted;
  runtimeTextChoice=LocalCombo(tc,TextModelNames);
  runtimeTextModelInfo=AddLabel(tc,L.T("Установка ещё не проверена","Installation has not been checked yet"),9,46);
  Button(Row(tc),L.T("Скачать выбранную для рантайма","Download selected runtime model"),async delegate {await LocalAction(async delegate {string key=SelectedTextModel(runtimeTextChoice);await EnsureBackend();await RunJob("install",new {component=key,gpu=true});});});
  textChoice.SelectedIndexChanged+=delegate {UpdateTextModelInfo();};runtimeTextChoice.SelectedIndexChanged+=delegate {UpdateTextModelInfo();};
  var textRuntime=Card(mp);componentLabels["llama-runtime"]=AddLabel(textRuntime,L.T("llama.cpp · среда текстовых моделей","llama.cpp · text model runtime"),11,45);
  Button(Row(textRuntime),L.T("Установить среду текста","Install text runtime"),async delegate {await LocalAction(async delegate {await EnsureBackend();await RunJob("install",new {component="llama-runtime"});});});
  var vc=Card(mp);AddLabel(vc,L.T("Голосовые модели","Speech models"),20,42).Font=new Font("Georgia",20);
  AddLabel(vc,L.T("Озвучивают готовые фразы на выбранном языке. Голосовые записи берутся из выбранного пользовательского пресета.","Read prepared phrases in the selected language. Voice references come from the selected personal preset."),10,54).ForeColor=muted;
  ttsChoice=LocalCombo(vc,new object[]{L.T("CosyVoice 3 0.5B · интонации · 5,43 ГБ","CosyVoice 3 0.5B · delivery control · 5.43 GB"),L.T("Qwen3-TTS 0.6B · 2,52 ГБ","Qwen3-TTS 0.6B · 2.52 GB"),L.T("Qwen3-TTS 1.7B · 4,54 ГБ","Qwen3-TTS 1.7B · 4.54 GB")});
  AddLabel(vc,L.T("Подача CosyVoice:","CosyVoice delivery:"),10,32).ForeColor=muted;deliveryChoice=LocalCombo(vc,DeliveryNames);
  AddLabel(vc,L.T("Название подачи (для добавления или изменения):","Delivery name (add or edit):"),9,27).ForeColor=muted;
  deliveryName=new TextBox {Width=595,MaxLength=60,BackColor=BackColor,ForeColor=ink};vc.Controls.Add(deliveryName);
  AddLabel(vc,L.T("Инструкция актёру · лучше на английском:","Acting instruction · English recommended:"),9,27).ForeColor=muted;
  deliveryInstruction=new TextBox {Width=595,Height=62,Multiline=true,MaxLength=600,BackColor=BackColor,ForeColor=ink};vc.Controls.Add(deliveryInstruction);
  deliveryChoice.SelectedIndexChanged+=delegate {ShowDelivery();};
  var dr=Row(vc);
  Button(dr,L.T("Добавить","Add"),async delegate {await LocalAction(async delegate {await SaveDelivery(true);});});
  Button(dr,L.T("Сохранить подачу","Save delivery"),async delegate {await LocalAction(async delegate {await SaveDelivery(false);});});
  Button(dr,L.T("Удалить подачу","Delete delivery"),async delegate {await LocalAction(DeleteDelivery);});
  AddLabel(vc,L.T("CosyVoice генерирует интонацию по команде: эмоцию, темп и мягкость. «Авто» просит текстовую модель подобрать подачу каждой новой фразе. Своя инструкция — до 600 символов, лучше по-английски, например: Speak warmly, slowly, with a subtle smile.","CosyVoice uses a separate instruction for emotion, pace and tone. Auto chooses a delivery for each phrase. Custom instructions: up to 600 characters; English recommended, e.g. Speak calmly, at an even pace."),9,86).ForeColor=muted;
  AddLabel(vc,L.T("Подача сохраняется с записью и учитывается в кэше. Изменения применяются при новой генерации или «Озвучить заново». У Qwen3-TTS Base эти настройки не меняют голос.","Delivery is saved with each recording and included in its cache key. Changes apply to new speech or Regenerate voice. Qwen3-TTS Base does not use these settings."),9,61).ForeColor=muted;
  foreach(string component in VoiceModelIds) {
   string captured=component; var card=Card(mp);
   componentLabels[component]=AddLabel(card,(component=="tts-cosyvoice3"?"CosyVoice 3":"Qwen3-TTS "+component.Replace("tts-",""))+L.T(" · ещё не проверена"," · not checked yet"),11,45);
   var r=Row(card);
   Button(r,L.T("Скачать голос ","Download speech model ")+(component=="tts-cosyvoice3"?"CosyVoice 3":component.Replace("tts-","")),async delegate {await LocalAction(async delegate {await EnsureBackend();await RunJob("install",new {component=captured,gpu=true});});});
  }
  var cosyCard=Card(mp);componentLabels["cosy-runtime"]=AddLabel(cosyCard,L.T("Среда CosyVoice · отдельные зависимости","CosyVoice runtime · separate dependencies"),11,45);
  Button(Row(cosyCard),L.T("Установить среду CosyVoice","Install CosyVoice runtime"),async delegate {await LocalAction(async delegate {await EnsureBackend();await RunJob("install",new {component="cosy-runtime",gpu=true});});});
  var rc=Card(mp);componentLabels["runtime"]=AddLabel(rc,L.T("Python / PyTorch · среда голосовых моделей","Python / PyTorch · speech runtime"),11,45);
  Button(Row(rc),L.T("Установить среду NVIDIA","Install NVIDIA runtime"),async delegate {await LocalAction(async delegate {await InstallRuntime(true);await RefreshComponents();});});
  Button(Row(rc),L.T("Установить среду CPU","Install CPU runtime"),async delegate {await LocalAction(async delegate {await InstallRuntime(false);await RefreshComponents();});});
  var dc=Card(mp);AddLabel(dc,L.T("Устройство для текста и голоса:","Device for text and speech:"),10,32).ForeColor=muted;
  deviceChoice=LocalCombo(dc,new object[]{L.T("Автоматически · NVIDIA при наличии CUDA","Automatic · NVIDIA when CUDA is available"),L.T("CPU · медленнее, без видеокарты","CPU · slower, no graphics card needed"),"NVIDIA CUDA"});
  Button(Row(dc),L.T("Применить выбор","Apply selection"),async delegate {await LocalAction(SaveLocalSettings);});
  var qc=Card(mp); modelInfo=AddLabel(qc,L.T("Среда CPU или NVIDIA выбирается по настройке устройства и поддержке драйвера. Готовые модели работают офлайн.","CPU or NVIDIA runtimes are selected using the device setting and driver support. Prepared models work offline."),9,65);modelInfo.ForeColor=muted;
  localProgress=AddLabel(qc,L.T("Очередь свободна","Queue is idle"),10,63); localBar=new ProgressBar {Width=595,Height=12,Minimum=0,Maximum=100}; qc.Controls.Add(localBar);
  var qr=Row(qc);Button(qr,L.T("Отменить очередь","Cancel queue"),async delegate {if(bootstrapCancel!=null) bootstrapCancel.Cancel();await LocalAction(async delegate {foreach(string id in activeJobs.ToArray()) await Api("/jobs/"+id,new {});});});
  Button(qr,L.T("Проверить компоненты","Check components"),async delegate {await LocalAction(async delegate {await EnsureBackend();await RefreshComponents();});});
  ms.SetContent(mp);
  dailyTimer=new System.Windows.Forms.Timer {Interval=30000};dailyTimer.Tick+=async delegate {await LocalAction(CheckDaily);};
  if(!Environment.GetCommandLineArgs().Contains("--preview")) Shown+=async delegate {
   await LocalAction(DesktopStartup);
  };
 }
 async Task LocalAction(Func<Task> action) {try {if(storageMoving)return;await action();} catch(Exception e) {if(!IsDisposed) {status.Text=e.Message;localProgress.Text=e.Message;localBar.Style=ProgressBarStyle.Blocks;}}}
 async Task ChangeStorage() {
  if(SharedStorage.IsIsolated(Environment.GetCommandLineArgs()))throw new Exception(L.T("Выбор общей папки доступен в обычном запуске приложения.","Shared storage can be changed during a normal application launch."));
  if(activeJobs.Count>0||runtimeInstalling||nativeTasks>0)throw new Exception(L.T("Сначала дождитесь очереди или отмените генерацию.","Wait for the queue or cancel generation first."));
  string destination;
  using(var picker=new FolderBrowserDialog {Description=L.T("Выберите пустую папку для моделей, библиотеки и среды Кавычки","Choose an empty folder for Kavychka models, library and runtimes"),SelectedPath=Folder,ShowNewFolderButton=true}) {if(picker.ShowDialog(this)!=DialogResult.OK)return;destination=picker.SelectedPath;}
  string previous=Folder;Exception failure=null;int remaining=0;bool finished=false,moved=false;
  storageMoving=true;
  try {
   tabs.Enabled=false;timer.Stop();dailyTimer.Stop();StopAudio();Save();PersistDesktopReminder(DateTime.UtcNow);
   if(backend!=null){if(!backend.HasExited){backend.Kill();backend.WaitForExit();}backend.Dispose();backend=null;}
   endpoint=null;backendStarting=null;if(localHttp!=null){localHttp.Dispose();localHttp=null;}
   using(var cancel=new CancellationTokenSource())using(var dialog=new Form {Text=L.T("Перенос хранилища","Move storage"),Width=560,Height=190,StartPosition=FormStartPosition.CenterParent,FormBorderStyle=FormBorderStyle.FixedDialog,MaximizeBox=false,MinimizeBox=false,BackColor=BackColor,ForeColor=ink}) {
    var message=new Label {Left=18,Top=18,Width=510,Height=53,Text=L.T("Подготовка проверенной копии…","Preparing a verified copy…")};dialog.Controls.Add(message);
    var bar=new ProgressBar {Left=18,Top=77,Width=505,Height=15};dialog.Controls.Add(bar);
    var stop=new Button {Left=18,Top=105,Width=160,Height=30,Text=L.T("Отменить перенос","Cancel move")};stop.Click+=delegate {cancel.Cancel();message.Text=L.T("Отмена… Старое хранилище сохраняется.","Cancelling… The original storage is kept.");};dialog.Controls.Add(stop);
    var progress=new Progress<Tuple<string,long,long>>(p=>{if(finished||dialog.IsDisposed)return;message.Text=p.Item1;bar.Value=p.Item3==0?0:(int)Math.Min(100,p.Item2*100/p.Item3);});
    dialog.FormClosing+=delegate(object sender,FormClosingEventArgs e){if(!finished){cancel.Cancel();e.Cancel=true;}};
    dialog.Shown+=async delegate {
     try {
      remaining=await Task.Run(delegate {return SharedStorage.Relocate(previous,destination,Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),cancel.Token,(m,d,t)=>((IProgress<Tuple<string,long,long>>)progress).Report(Tuple.Create(m,d,t)),delegate(string target) {
       if(!File.Exists(Path.Combine(target,"library.sqlite3")))return;
       string python=Environment.GetEnvironmentVariable("KAVYCHKA_PYTHON");if(string.IsNullOrWhiteSpace(python))python=Path.Combine(target,"runtime","python.exe");
       var info=new ProcessStartInfo(python,"-B \""+Path.Combine(Application.StartupPath,"backend","relocate_storage.py")+"\" --source \""+previous+"\" --target \""+target+"\"") {UseShellExecute=false,CreateNoWindow=true,RedirectStandardOutput=true,RedirectStandardError=true};
       using(var process=Process.Start(info)){process.StandardOutput.ReadToEnd();string reason=process.StandardError.ReadToEnd();process.WaitForExit();if(process.ExitCode!=0)throw new IOException(L.T("Не пройдена проверка библиотеки: ","Library verification failed: ")+reason);}
      });});moved=true;
     }catch(Exception e){failure=e;}finally {finished=true;dialog.Close();}
    };
    dialog.ShowDialog(this);
   }
   if(moved){Folder=Path.GetFullPath(destination);config=Path.Combine(Folder,"settings.json");storagePrepared=true;pendingStorage=new string[0];storageInfo.Text=L.T("Общее хранилище для всех версий:\r\n","Shared storage for all versions:\r\n")+Folder;}
  }finally {storageMoving=false;tabs.Enabled=true;timer.Start();}
  if(File.Exists(PythonPath())){await EnsureBackend();await RefreshComponents();await RefreshLibrary();}
  if(failure is OperationCanceledException){status.Text=L.T("Перенос отменён. Старое хранилище сохранено.","Move cancelled. The original storage is kept.");return;}
  if(failure!=null)throw failure;
  status.Text=remaining==0?L.T("Хранилище перенесено. Модели и записи доступны, новый путь сохранён для обновлений.","Storage moved. Models and recordings are available; updates will use the saved path."):L.T("Хранилище переключено. Часть файлов осталась в старой папке: они были заняты или изменились.","Storage changed. Some files remain in the original folder because they were busy or changed.");
 }
 string PythonPath() {string custom=Environment.GetEnvironmentVariable("KAVYCHKA_PYTHON");return string.IsNullOrWhiteSpace(custom)?Path.Combine(Folder,"runtime","python.exe"):custom;}
 async Task PrepareStorage() {
  if(storagePrepared)return;if(storagePreparing!=null){await storagePreparing;return;}
  storagePreparing=StageStorage();try {await storagePreparing;storagePrepared=true;}finally {storagePreparing=null;}
 }
 async Task StageStorage() {
  if(SharedStorage.IsIsolated(Environment.GetCommandLineArgs()))return;
  pendingStorage=SharedStorage.Pending(Application.StartupPath,Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),Folder);
  if(pendingStorage.Length==0)return;
  bootstrapCancel=new CancellationTokenSource();var cancel=bootstrapCancel.Token;
  var progress=new Progress<Tuple<string,long,long>>(update=>{if(IsDisposed)return;status.Text=localProgress.Text=update.Item1;localBar.Style=ProgressBarStyle.Blocks;localBar.Value=update.Item3==0?0:(int)Math.Min(100,update.Item2*100/update.Item3);});
  await Task.Run(delegate {SharedStorage.Stage(Folder,pendingStorage,cancel,(message,done,total)=>((IProgress<Tuple<string,long,long>>)progress).Report(Tuple.Create(message,done,total)));},cancel);
 }
 async Task ImportStorage() {
  if(pendingStorage.Length==0)return;
  string list=Path.Combine(Folder,"migration-pending.json");File.WriteAllText(list,Json.Serialize(pendingStorage),new UTF8Encoding(false));
  localProgress.Text=status.Text=L.T("Переношу библиотеку, настройки и историю воспроизведения…","Moving the library, preferences and playback history…");
  var info=new ProcessStartInfo(PythonPath(),"-B \""+Path.Combine(Application.StartupPath,"backend","migrate_storage.py")+"\" --target \""+Folder+"\" --sources \""+list+"\"") {UseShellExecute=false,CreateNoWindow=true,RedirectStandardOutput=true,RedirectStandardError=true,StandardOutputEncoding=Encoding.UTF8,StandardErrorEncoding=Encoding.UTF8};
  info.EnvironmentVariables["PYTHONIOENCODING"]="utf-8";
  using(var process=Process.Start(info)) {
   Task<string> output=process.StandardOutput.ReadToEndAsync(),error=process.StandardError.ReadToEndAsync();await Task.Run(delegate {process.WaitForExit();});await output;string reason=await error;
   if(process.ExitCode!=0)throw new Exception(L.T("Перенос библиотеки не завершён. Старые данные сохранены. ","Library migration did not complete. Original data is kept. ")+reason);
  }
  SharedStorage.Complete(Folder,pendingStorage);pendingStorage=new string[0];File.Delete(list);
  string warnings=Path.Combine(Folder,"migration-warnings.json");
  if(File.Exists(warnings))MessageBox.Show(this,string.Join("\r\n",Json.Deserialize<string[]>(File.ReadAllText(warnings,Encoding.UTF8))),L.T("Перенос компонентов","Component migration"),MessageBoxButtons.OK,MessageBoxIcon.Warning);
 }
 async Task VerifySharedRuntime(bool force) {
  string marker=Path.Combine(Folder,"runtime-ok.json");
  if((File.Exists(marker)&&!force)||!Directory.Exists(Path.Combine(Folder,"runtime","Lib","site-packages","torch")))return;
  localProgress.Text=status.Text=L.T("Проверяю перенесённую среду Python и PyTorch…","Checking the migrated Python and PyTorch runtime…");
  var info=new ProcessStartInfo(PythonPath(),"-B -c \"import torch,torchaudio,qwen_tts,soundfile,transformers,tzdata; print(torch.__version__)\"") {UseShellExecute=false,CreateNoWindow=true,RedirectStandardOutput=true,RedirectStandardError=true,StandardOutputEncoding=Encoding.UTF8,StandardErrorEncoding=Encoding.UTF8};
  info.EnvironmentVariables["PYTHONIOENCODING"]="utf-8";
  using(var process=Process.Start(info)) {
   Task<string> output=process.StandardOutput.ReadToEndAsync(),error=process.StandardError.ReadToEndAsync();await Task.Run(delegate {process.WaitForExit();});string version=await output;await error;
   if(process.ExitCode==0)File.WriteAllText(marker,Json.Serialize(new {verified=true,torch=version.Trim()}));
   else {if(File.Exists(marker))File.Delete(marker);status.Text=L.T("Файлы перенесены. Среду нужно восстановить кнопкой установки; модели сохранены.","Files migrated. Use the install button to repair the runtime; models are kept.");}
  }
 }
 async Task EnsureBackend() {
  if(endpoint!=null && backend!=null && !backend.HasExited) return;
  if(backendStarting!=null) {await backendStarting;return;}
  backendStarting=StartBackend();try {await backendStarting;} finally {backendStarting=null;}
 }
 async Task StartBackend() {
  await PrepareStorage();
  if(!File.Exists(PythonPath())) await BootstrapPython();
  bool imported=pendingStorage.Length>0;
  await ImportStorage();
  await VerifySharedRuntime(imported);
  string token=Guid.NewGuid().ToString("N")+Guid.NewGuid().ToString("N");
  var ready=new TaskCompletionSource<int>(); string lastError="";
  var info=new ProcessStartInfo(PythonPath(),"-B \""+Path.Combine(Application.StartupPath,"backend","server.py")+"\" --root \""+Application.StartupPath+"\" --data \""+Folder+"\" --parent "+Process.GetCurrentProcess().Id) {UseShellExecute=false,CreateNoWindow=true,RedirectStandardOutput=true,RedirectStandardError=true,StandardOutputEncoding=Encoding.UTF8,StandardErrorEncoding=Encoding.UTF8,WorkingDirectory=Application.StartupPath};
  info.EnvironmentVariables["KAVYCHKA_TOKEN"]=token; info.EnvironmentVariables["PYTHONIOENCODING"]="utf-8";
  backend=new Process {StartInfo=info,EnableRaisingEvents=true};
  backend.OutputDataReceived+=delegate(object sender,DataReceivedEventArgs e) {if(e.Data!=null && e.Data.StartsWith("{\"port\"")) try {ready.TrySetResult(Convert.ToInt32(D(new System.Web.Script.Serialization.JavaScriptSerializer().DeserializeObject(e.Data))["port"]));} catch {}};
  backend.ErrorDataReceived+=delegate(object sender,DataReceivedEventArgs e) {if(e.Data!=null) lastError=e.Data;};
  backend.Exited+=delegate {ready.TrySetException(new Exception(L.T("Локальный процесс завершился: ","The local process exited: ")+lastError));};
  backend.Start();backend.BeginOutputReadLine();backend.BeginErrorReadLine();
  if(await Task.WhenAny(ready.Task,Task.Delay(30000))!=ready.Task) {backend.Kill();throw new Exception(L.T("Локальный процесс не запустился за 30 секунд.","The local process did not start within 30 seconds."));}
  int port=await ready.Task;endpoint="http://127.0.0.1:"+port;
  if(localHttp!=null)localHttp.Dispose();localHttp=new HttpClient(new HttpClientHandler {UseProxy=false}) {Timeout=TimeSpan.FromSeconds(120)};
  localHttp.DefaultRequestHeaders.Authorization=new AuthenticationHeaderValue("Bearer",token);
  dailyTimer.Start();
 }
 async Task<Dictionary<string,object>> Api(string path,object payload=null) {
  await EnsureBackend();
  using(var request=new HttpRequestMessage(payload==null?HttpMethod.Get:HttpMethod.Post,endpoint+path)) {
   request.Headers.Add("X-Kavychka-Language",settings.UiLanguage);
   if(payload!=null) request.Content=new StringContent(Json.Serialize(payload),Encoding.UTF8,"application/json");
   using(var response=await localHttp.SendAsync(request)) {var obj=D(Json.DeserializeObject(await response.Content.ReadAsStringAsync()));if(!response.IsSuccessStatusCode) throw new Exception(S(obj,"error"));return obj;}
  }
 }
 async Task<byte[]> ApiBytes(string path,object payload=null) {
  await EnsureBackend();using(var request=new HttpRequestMessage(payload==null?HttpMethod.Get:HttpMethod.Post,endpoint+path)) {
   request.Headers.Add("X-Kavychka-Language",settings.UiLanguage);
   if(payload!=null) request.Content=new StringContent(Json.Serialize(payload),Encoding.UTF8,"application/json");
   using(var response=await localHttp.SendAsync(request)) {if(!response.IsSuccessStatusCode) throw new Exception(S(D(Json.DeserializeObject(await response.Content.ReadAsStringAsync())),"error"));return await response.Content.ReadAsByteArrayAsync();}
  }
 }
 void MigrateBackendSettings(Dictionary<string,object> s){
  bool changed=false;
  if(!settings.BackendPersonalizationMigrated){string description=S(s,"character_description");if(!PersonalPresets.IsDefaultDescription(Application.StartupPath,description)&&!string.IsNullOrWhiteSpace(description))settings.CharacterFile=LegacyPersonalization.SaveCharacter(Folder,description,L.T("Мой прежний персонаж","My previous character"));settings.BackendPersonalizationMigrated=true;changed=true;}
  // The database owns advanced settings; the desktop file owns shared choices
  // after this one-time import. A late startup response must not undo a UI save.
  if(!settings.BackendSettingsMigrated){
   var overrides=settings.BackendSettingsOverrides??new List<string>();
   if(!overrides.Contains("text_model")&&TextModelIds.Contains(S(s,"text_model")))settings.TextModel=S(s,"text_model");
   if(!overrides.Contains("runtime_text_model")&&TextModelIds.Contains(S(s,"runtime_text_model")))settings.RuntimeTextModel=S(s,"runtime_text_model");
   if(!overrides.Contains("tts_model")&&VoiceModelIds.Contains(S(s,"tts_model")))settings.VoiceModel=S(s,"tts_model");
   if(!overrides.Contains("swear"))settings.Swear=B(s,"swear");
   if(!overrides.Contains("runtime"))settings.RuntimeGeneration=B(s,"runtime");
   settings.BackendSettingsMigrated=true;changed=true;
  }
  if(!settings.BackendDailySettingsMigrated){var overrides=settings.BackendSettingsOverrides??new List<string>();if(!overrides.Contains("auto_daily"))settings.AutomaticPreparation=B(s,"auto_daily");if(!overrides.Contains("daily_auto_keep"))settings.AutoKeepDaily=B(s,"daily_auto_keep");settings.BackendDailySettingsMigrated=true;changed=true;}
  if(settings.BackendSettingsMigrated&&settings.BackendDailySettingsMigrated)settings.BackendSettingsOverrides=new List<string>();
  if(changed){Save();RefreshSharedModelControls();if(simpleSettings!=null&&!simpleSettings.IsDisposed){simpleSettings.RebaseModelChoices(settings);simpleSettings.RebaseCharacterChoice(settings,PersonalPresets.List(Folder));}}
 }
 void RefreshSharedModelControls(){
  bool wasLoading=localLoading;localLoading=true;
  try{runtimeToggle.Checked=settings.RuntimeGeneration;localSwear.Checked=settings.Swear;autoDaily.Checked=settings.AutomaticPreparation;autoKeep.Checked=settings.AutoKeepDaily;ttsChoice.SelectedIndex=Math.Max(0,Array.IndexOf(VoiceModelIds,settings.VoiceModel));textChoice.SelectedIndex=Math.Max(0,Array.IndexOf(TextModelIds,settings.TextModel));runtimeTextChoice.SelectedIndex=Math.Max(0,Array.IndexOf(TextModelIds,settings.RuntimeTextModel));UpdateTextModelInfo();}
  finally{localLoading=wasLoading;}
 }
 async Task LoadLocalSettings() {
  var s=await Api("/settings");MigrateBackendSettings(s);localLoading=true;
  try {RefreshSharedModelControls();phraseCount.Value=Convert.ToDecimal(s["batch_count"]);deviceChoice.SelectedIndex=S(s,"device")=="cpu"?1:S(s,"device")=="cuda"?2:0;
   deliveryCatalog.Clear();foreach(object item in (object[])s["delivery_catalog"])deliveryCatalog.Add(D(item));PopulateDeliveries(S(s,"cosy_style"));
   string selectedCharacter=PersonalPresets.Description(Folder,settings.CharacterFile);characterDescription.Text=CharacterLines(selectedCharacter.Length==0?defaultCharacter:selectedCharacter);
   var cats=((object[])s["categories"]).Select(Convert.ToString).ToArray();for(int i=0;i<Categories.Length;i++) categoryChoice.SetItemChecked(i,cats.Contains(Categories[i]));
  } finally {localLoading=false;}
 }
 async Task SaveCharacter() {
  await Api("/settings",new {character_description=characterDescription.Text});
  settings.CharacterFile=LegacyPersonalization.SaveCharacter(Folder,characterDescription.Text,L.T("Мой персонаж","My character"));settings.BackendPersonalizationMigrated=true;if(!Save())throw new IOException(settingsSaveError);
  status.Text=L.T("Описание сохранено. Оно применяется к новым фразам; готовые записи остаются в библиотеке.","Description saved. It applies to new phrases; existing recordings remain in the library.");
 }
 async Task SaveLocalSettings() {
  if(localLoading)return;
  var cats=Enumerable.Range(0,Categories.Length).Where(i=>categoryChoice.GetItemChecked(i)).Select(i=>Categories[i]).ToArray();
  await Api("/settings",new {runtime=runtimeToggle.Checked,auto_daily=autoDaily.Checked,daily_auto_keep=autoKeep.Checked,swear=localSwear.Checked,batch_count=(int)phraseCount.Value,categories=cats,tts_model=VoiceModelIds[Math.Max(0,ttsChoice.SelectedIndex)],cosy_style=DeliveryIds[Math.Max(0,deliveryChoice.SelectedIndex)],cosy_instruction=deliveryInstruction.Text,text_model=SelectedTextModel(textChoice),runtime_text_model=SelectedTextModel(runtimeTextChoice),device=deviceChoice.SelectedIndex==1?"cpu":deviceChoice.SelectedIndex==2?"cuda":"auto"});
  settings.TextModel=SelectedTextModel(textChoice);settings.RuntimeTextModel=SelectedTextModel(runtimeTextChoice);settings.VoiceModel=VoiceModelIds[Math.Max(0,ttsChoice.SelectedIndex)];
  settings.Swear=localSwear.Checked;settings.RuntimeGeneration=runtimeToggle.Checked;settings.AutomaticPreparation=autoDaily.Checked;settings.AutoKeepDaily=autoKeep.Checked;settings.BackendSettingsMigrated=true;settings.BackendDailySettingsMigrated=true;settings.BackendSettingsOverrides=new List<string>();if(!Save())throw new IOException(settingsSaveError);status.Text=L.T("Локальные настройки сохранены.","Local settings saved.");
 }
 async Task RefreshComponents() {
  var result=await Api("/status");
  ReadHardwareStatus(result);
  foreach(object raw in (object[])result["components"]) {var c=D(raw);string id=S(c,"id");componentStates[id]=S(c,"name")+"\r\n"+(B(c,"installed")?L.T("Установлено · работает офлайн","Installed · works offline"):L.T("Нужно скачать · ","Download needed · ")+(Convert.ToDouble(c["bytes"])/1e9).ToString("0.00")+L.T(" ГБ"," GB"));if(componentLabels.ContainsKey(id))componentLabels[id].Text=componentStates[id];}
  UpdateTextModelInfo();
 }
 void ReadHardwareStatus(Dictionary<string,object> result){detectedHardware=result.ContainsKey("hardware")?D(result["hardware"]):null;installationDevices=result.ContainsKey("installation")?D(result["installation"]):null;if(simpleSettings!=null&&!simpleSettings.IsDisposed)simpleSettings.UpdateHardware(detectedHardware,installationDevices);}
 async Task RefreshLibrary() {
  var result=await Api("/library");var ids=new HashSet<string>(SelectedIds());library.BeginUpdate();library.Items.Clear();
  int available=0,todayCount=0;
  foreach(object raw in (object[])result["rows"]) {
   var row=D(raw);bool audio=S(row,"audio_path").Length>0,used=S(row,"used_at").Length>0,approved=B(row,"approved");
   if(S(row,"day")==Today()){todayCount++;if(audio&&approved&&!used)available++;}
   int category=Array.IndexOf(Categories,S(row,"category"));
   DateTime date;string day=DateTime.TryParse(S(row,"day"),out date)?date.ToString("dd.MM.yyyy"):S(row,"day");
   int delivery=Array.IndexOf(DeliveryIds,S(row,"voice_style"));string deliveryName=S(row,"model")=="tts-cosyvoice3"&&delivery>=0?Convert.ToString(DeliveryNames[delivery]):"—";
   var item=new ListViewItem(new[]{day,category<0?"?":CategoryNames[category],S(row,"model").Replace("tts-",""),deliveryName,!audio?L.T("Без аудио","No audio"):used?L.T("Проиграна","Played"):approved?L.T("Оставлена","Approved"):L.T("Кандидат","Candidate"),S(row,"text")}); item.Tag=row;library.Items.Add(item);item.Selected=ids.Contains(S(row,"id"));
  }
  library.EndUpdate();var stats=D((await Api("/status"))["stats"]);
  libraryInfo.Text=L.T("Сегодня: ","Today: ")+todayCount+L.T(" фраз · готовы без повторов: "," phrases · unused and ready: ")+available+L.T(" · аудио: "," · audio: ")+(Convert.ToDouble(stats["audio_bytes"])/1048576).ToString("0.0")+L.T(" МБ"," MB");
 }
 string[] SelectedIds() {return library.SelectedItems.Cast<ListViewItem>().Select(i=>S(D(i.Tag),"id")).ToArray();}
 async Task ClearLibrary() {
  if(activeJobs.Count>0||runtimeInstalling)throw new Exception(L.T("Дождитесь завершения очереди или отмените её перед очисткой.","Wait for the queue or cancel it before clearing."));
  if(MessageBox.Show(this,L.T("Удалить ВСЕ фразы, историю воспроизведения, планы и аудиозаписи?\r\nМодели и настройки сохранятся. Отменить удаление нельзя.","Delete ALL phrases, playback history, plans and recordings?\r\nModels and settings are kept. This cannot be undone."),L.T("Очистить библиотеку","Clear library"),MessageBoxButtons.YesNo,MessageBoxIcon.Warning,MessageBoxDefaultButton.Button2)!=DialogResult.Yes)return;
  StopAudio();await Api("/library/clear",new {confirm="clear-library"});dailyAttempt=Today();selectedText.Clear();await RefreshLibrary();
  status.Text=L.T("Библиотека очищена. Для новых фраз нажмите «Новая партия».","Library cleared. Click New batch to create new phrases.");
 }
 async Task SelectLocal(string action) {await LocalAction(async delegate {string[] ids=SelectedIds();if(ids.Length==0)throw new Exception(L.T("Выберите строки в библиотеке.","Select rows in the library."));await Api("/select",new {ids=ids,action=action});await RefreshLibrary();});}
 async Task PreviewSelected() {string[] ids=SelectedIds();if(ids.Length==0)throw new Exception(L.T("Выберите запись для прослушивания.","Select a recording to preview."));StopAudio();int version=audioVersion;await PlayLocal(ids[0],version);}
 async Task PlayLocal(string id,int version,bool automatic=false) {
  byte[] bytes=await ApiBytes("/audio/"+id);if(version!=audioVersion||IsDisposed||exiting||(automatic&&(settings.Paused||!Schedule.IsWorkday(DateTime.UtcNow))))return;
  audioStream=new MemoryStream(bytes);player=new System.Media.SoundPlayer(audioStream);player.Load();player.Play();if(pet!=null){pet.Speaking=true;FinishPetSpeech(version,MediaTools.WaveDuration(bytes));}status.Text=L.T("Воспроизводится локальная запись.","Playing a local recording.");
 }
 async Task ExportLocal() {using(var dialog=new SaveFileDialog {Filter=L.T("ZIP архив|*.zip","ZIP archive|*.zip"),FileName="Kavychka-"+Today()+".zip"}) {if(dialog.ShowDialog(this)!=DialogResult.OK)return;byte[] zip=await ApiBytes("/export",new {ids=SelectedIds()});await Task.Run(delegate {File.WriteAllBytes(dialog.FileName,zip);});status.Text=L.T("Архив сохранён: ","Archive saved: ")+dialog.FileName;}}
 async Task<Dictionary<string,object>> RunJob(string kind,object args,Action<Dictionary<string,object>> partial=null) {
  var job=await Api("/jobs",new {kind=kind,args=args});string id=S(job,"id");if(id.Length==0)return D(job["result"]);
  activeJobs.Add(id);string seen="";
  try {
   while(true) {
    if(IsDisposed||exiting)return null;
    string state=S(job,"state"),message=S(job,"message");status.Text=message;localProgress.Text=message;
    double total=Convert.ToDouble(job["total"]);localBar.Style=total>0?ProgressBarStyle.Blocks:ProgressBarStyle.Marquee;if(total>0)localBar.Value=(int)Math.Max(0,Math.Min(100,Convert.ToDouble(job["done"])*100/total));
    var row=D(job["result"]);if(partial!=null&&row!=null&&S(row,"text").Length>0&&seen!=S(row,"id")){seen=S(row,"id");partial(row);}
    if(state=="done") {localBar.Style=ProgressBarStyle.Blocks;localBar.Value=100;await RefreshLibrary();await RefreshComponents();return row;}
    if(state=="failed"||state=="cancelled")throw new Exception(S(job,"error"));
    await Task.Delay(1000);job=await Api("/jobs/"+id);
   }
  } finally {activeJobs.Remove(id);}
 }
 async Task CheckDaily() {
  await PrepareDesktopDay();
 }
 async Task RemindLocal(bool exercise,string meal,bool automatic) {
  if(storageMoving)return;
  if(automatic&&(settings.Paused||!Schedule.IsWorkday(DateTime.UtcNow)))return;
  string category=meal??(exercise?"exercise":"break");StopAudio();int version=audioVersion;
  string initial=meal==null?(exercise?"Время немного размяться.":"Пора сделать перерыв."):Schedule.MealText(meal);
  Form popup=ShowReminder(initial,exercise,meal);
  DateTime signalAt=DateTime.UtcNow;
  if(settings.NotificationSound)NotificationChime.Play();
  try {
   if(runtimeInstalling)throw new Exception(L.T("Сейчас устанавливается среда. Генерация станет доступна после установки.","The runtime is being installed. Generation becomes available when it finishes."));
   var s=await Api("/settings");Dictionary<string,object> row;
   if(B(s,"runtime")) {
    row=await RunJob("runtime",new {day=Today(),category=category},delegate(Dictionary<string,object> partial) {UpdateLocalPopup(popup,S(partial,"text"));});
   } else row=D((await Api("/consume",new {day=Today(),category=category}))["row"]);
   if(row==null){status.Text=L.T("На сегодня нет неиспользованной разрешённой записи категории «","No unused approved recording is available today for category “")+CategoryNames[Array.IndexOf(Categories,category)]+L.T("». Подготовьте день или оставьте кандидатов.","”. Prepare the day or approve candidates.");return;}
   lastText.Text=S(row,"text")+(exercise&&meal!=null?"\r\nИ ещё: пришло время зарядки после завтрака.":"");UpdateLocalPopup(popup,lastText.Text);
   if(settings.NotificationSound) {int delay=1200-(int)(DateTime.UtcNow-signalAt).TotalMilliseconds;if(delay>0)await Task.Delay(delay);}
   if(version==audioVersion&&!popup.IsDisposed&&settings.Voice&&!(automatic&&settings.Paused))await PlayLocal(S(row,"id"),version,automatic);
   await RefreshLibrary();
  } catch(Exception e) {status.Text=L.T("Локальная генерация: ","Local generation: ")+e.Message;UpdateLocalPopup(popup,initial+L.T("\r\nЗапись недоступна. Подробности — в главном окне.","\r\nRecording unavailable. Details are in the main window."));}
 }
 void UpdateLocalPopup(Form popup,string text) {if(popup.IsDisposed)return;foreach(Control control in popup.Controls)if(control is TextBox)control.Text=text;lastText.Text=text;}
 async Task InstallRuntime(bool gpu) {
  if(activeJobs.Count>0||runtimeInstalling)throw new Exception(L.T("Сначала дождитесь завершения очереди или отмените её.","Wait for the queue or cancel it first."));
  runtimeInstalling=true;
  try {
   await EnsureBackend();
   if(backend!=null&&!backend.HasExited){backend.Kill();backend.WaitForExit(3000);backend.Dispose();backend=null;}
   endpoint=null;if(localHttp!=null){localHttp.Dispose();localHttp=null;}
   await EnsureBackend();await RunJob("install",new {component="runtime",gpu=gpu});
  } finally {runtimeInstalling=false;}
 }
 void CloseLocalAi() {
  if(dailyTimer!=null)dailyTimer.Dispose();if(bootstrapCancel!=null)bootstrapCancel.Cancel();
  if(backend!=null){try{if(!backend.HasExited){backend.Kill();backend.WaitForExit(5000);}}catch{}backend.Dispose();}if(localHttp!=null)localHttp.Dispose();
 }
 async Task BootstrapPython() {
  bootstrapCancel=new CancellationTokenSource();CancellationToken cancel=bootstrapCancel.Token;
  var manifest=D(Json.DeserializeObject(File.ReadAllText(Path.Combine(Application.StartupPath,"assets","components.json"))));var spec=D(manifest["python"]);
  string directory=Path.Combine(Folder,"runtime"),archive=Path.Combine(Folder,"python.zip.part");Directory.CreateDirectory(Folder);
  long offset=File.Exists(archive)?new FileInfo(archive).Length:0;
  using(var client=new HttpClient {Timeout=TimeSpan.FromMinutes(10)}) using(var request=new HttpRequestMessage(HttpMethod.Get,S(spec,"url"))) {
   if(offset>0)request.Headers.Range=new RangeHeaderValue(offset,null);
   using(var response=await client.SendAsync(request,HttpCompletionOption.ResponseHeadersRead,cancel)) {
    if(response.StatusCode==HttpStatusCode.RequestedRangeNotSatisfiable){File.Delete(archive);throw new Exception(L.T("Повторите установку среды: временный архив сброшен.","Retry runtime installation; the temporary archive was reset."));}
    response.EnsureSuccessStatusCode();if(response.StatusCode!=HttpStatusCode.PartialContent)offset=0;
    using(var source=await response.Content.ReadAsStreamAsync()) using(var file=new FileStream(archive,offset>0?FileMode.Append:FileMode.Create,FileAccess.Write,FileShare.None,65536,true)) {
     byte[] buffer=new byte[65536];int n;while((n=await source.ReadAsync(buffer,0,buffer.Length,cancel))>0){await file.WriteAsync(buffer,0,n,cancel);offset+=n;localProgress.Text="Python: "+(offset/1048576.0).ToString("0.0")+L.T(" МБ"," MB");localBar.Style=ProgressBarStyle.Blocks;localBar.Value=(int)Math.Min(100,offset*100/Convert.ToInt64(spec["size"]));}
    }
   }
  }
  localProgress.Text=L.T("Проверка Python и распаковка…","Verifying and unpacking Python…");
  await Task.Run(delegate {
   cancel.ThrowIfCancellationRequested();string hash;using(var sha=SHA256.Create())using(var stream=File.OpenRead(archive))hash=BitConverter.ToString(sha.ComputeHash(stream)).Replace("-","").ToLowerInvariant();
   if(hash!=S(spec,"sha256")){File.Delete(archive);throw new Exception(L.T("SHA-256 архива Python не совпадает. Повторите загрузку.","The Python archive SHA-256 does not match. Download it again."));}
   Directory.CreateDirectory(directory);
   using(var zip=ZipFile.OpenRead(archive))foreach(var entry in zip.Entries){cancel.ThrowIfCancellationRequested();string target=Path.GetFullPath(Path.Combine(directory,entry.FullName));if(!target.StartsWith(Path.GetFullPath(directory)+Path.DirectorySeparatorChar,StringComparison.OrdinalIgnoreCase))throw new Exception(L.T("Некорректный путь в архиве Python","Invalid path in the Python archive"));if(entry.Name.Length==0)Directory.CreateDirectory(target);else {Directory.CreateDirectory(Path.GetDirectoryName(target));entry.ExtractToFile(target,true);}}
   Directory.CreateDirectory(Path.Combine(directory,"Lib","site-packages"));File.WriteAllText(Path.Combine(directory,"python312._pth"),"python312.zip\r\n.\r\nLib/site-packages\r\nimport site\r\n",new UTF8Encoding(false));
  },cancel);
  File.Delete(archive);localProgress.Text=L.T("Python установлен. Установка библиотек…","Python installed. Installing libraries…");
 }
}
