using System;
using System.Collections;
using System.Collections.Generic;
using System.Drawing;
using System.IO;
using System.Linq;
using System.Net;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Security.Cryptography;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Web.Script.Serialization;
using System.Windows.Forms;
using System.Media;
using System.Runtime.InteropServices;
using System.Text.RegularExpressions;

public class NightForm : Form {
 [DllImport("user32.dll")] static extern bool ReleaseCapture();
 [DllImport("user32.dll")] static extern IntPtr SendMessage(IntPtr h, int message, IntPtr w, IntPtr l);
 protected override void OnLoad(EventArgs e) {
  // A custom caption shares the selected app palette.
  FormBorderStyle = FormBorderStyle.None; Padding = new Padding(1,39,1,1);
  var caption = new Panel { BackColor = StudioColors.Background, Location = new Point(1,1), Size = new Size(ClientSize.Width-2,36), Anchor = AnchorStyles.Top | AnchorStyles.Left | AnchorStyles.Right };
  var name = new Label { Text = Text, ForeColor = StudioColors.Muted, Location = new Point(14,8), Size = new Size(Math.Max(100,ClientSize.Width-160),24), Anchor = AnchorStyles.Left | AnchorStyles.Top | AnchorStyles.Right, Font = new Font("Segoe UI",9) };
  MouseEventHandler drag = delegate(object sender, MouseEventArgs m) { if (m.Button == MouseButtons.Left) { ReleaseCapture(); SendMessage(Handle,0xA1,new IntPtr(2),IntPtr.Zero); } };
  caption.MouseDown += drag; name.MouseDown += drag; caption.Controls.Add(name);
  var close = new Button { Text = "×", FlatStyle = FlatStyle.Flat, ForeColor = StudioColors.Ink, BackColor = caption.BackColor, Size = new Size(42,34), Location = new Point(caption.Width-43,0), Anchor = AnchorStyles.Top | AnchorStyles.Right, Font = new Font("Segoe UI",14), TabStop = false }; close.FlatAppearance.BorderSize = 0; close.FlatAppearance.MouseOverBackColor = StudioColors.Soft; close.Click += delegate { Close(); }; caption.Controls.Add(close);
  if (MinimizeBox) { var min = new Button { Text = "—", FlatStyle = FlatStyle.Flat, ForeColor = name.ForeColor, BackColor = caption.BackColor, Size = new Size(42,34), Location = new Point(caption.Width-85,0), Anchor = AnchorStyles.Top | AnchorStyles.Right, TabStop = false }; min.FlatAppearance.BorderSize = 0; min.Click += delegate { WindowState = FormWindowState.Minimized; }; caption.Controls.Add(min); }
  Controls.Add(caption); caption.BringToFront();
  base.OnLoad(e);
 }
 protected override void WndProc(ref Message m) {
  base.WndProc(ref m);
  if (m.Msg == 0x84 && MaximizeBox && WindowState == FormWindowState.Normal) {
   Point p = PointToClient(Cursor.Position); bool l=p.X<7, r=p.X>=ClientSize.Width-7, t=p.Y<7, b=p.Y>=ClientSize.Height-7;
   int hit = t && l ? 13 : t && r ? 14 : b && l ? 16 : b && r ? 17 : l ? 10 : r ? 11 : t ? 12 : b ? 15 : 0;
   if (hit != 0) m.Result = new IntPtr(hit);
  }
 }
 [DllImport("dwmapi.dll")] static extern int DwmSetWindowAttribute(IntPtr h, int attr, ref int value, int size);
 protected override void OnHandleCreated(EventArgs e) {
  base.OnHandleCreated(e);
  try { int yes = 1; if (DwmSetWindowAttribute(Handle,20,ref yes,4) != 0) DwmSetWindowAttribute(Handle,19,ref yes,4);
   int caption = 0x201D1B, text = 0xDBE6EE; DwmSetWindowAttribute(Handle,35,ref caption,4); DwmSetWindowAttribute(Handle,36,ref text,4);
  } catch (DllNotFoundException) { } catch (EntryPointNotFoundException) { }
 }
 public static void Notice(IWin32Window owner, string text, string title) {
  using (var f = new NightForm { Text = title, ClientSize = new Size(510,280), BackColor = StudioColors.Background, ForeColor = StudioColors.Ink, StartPosition = FormStartPosition.CenterParent, Font = new Font("Segoe UI",11), MinimizeBox = false, MaximizeBox = false }) {
   f.Controls.Add(new Label { Text = text, Location = new Point(24,60), Size = new Size(462,145) });
   var b = new StudioButton { Text = L.T("Понятно","Got it"), DialogResult = DialogResult.OK, Location = new Point(355,230), Size = new Size(130,35) }; f.Controls.Add(b); f.AcceptButton = b; f.CancelButton = b; f.ShowDialog(owner);
  }
 }
}
public class NightTabs : TabControl {
 public NightTabs() { SetStyle(ControlStyles.UserPaint | ControlStyles.AllPaintingInWmPaint | ControlStyles.OptimizedDoubleBuffer, true); SizeMode = TabSizeMode.Fixed; ItemSize = new Size(215,46); Padding = new Point(12,8); }
 protected override void OnSelectedIndexChanged(EventArgs e) { base.OnSelectedIndexChanged(e); Invalidate(); }
 protected override void OnPaint(PaintEventArgs e) {
  e.Graphics.Clear(StudioColors.Background);
  for (int i=0;i<TabCount;i++) { Rectangle r = GetTabRect(i); bool selected = i == SelectedIndex;
   TextRenderer.DrawText(e.Graphics,TabPages[i].Text,Font,r,selected ? StudioColors.Ink : StudioColors.Muted,TextFormatFlags.HorizontalCenter | TextFormatFlags.VerticalCenter);
   if (selected) using (var brush = new SolidBrush(StudioColors.Accent)) e.Graphics.FillRectangle(brush,r.Left+15,r.Bottom-3,r.Width-30,3);
   if (selected && Focused) ControlPaint.DrawFocusRectangle(e.Graphics,Rectangle.Inflate(r,-6,-6));
  }
 }
}
public class NightScroll : Panel {
 int offset;
 bool dragging;
 Control content;
 readonly Control rail;
 public Control Content {get{return content;}}
 sealed class ScrollRail:Control {
  readonly NightScroll owner;bool held;
  public ScrollRail(NightScroll owner){this.owner=owner;DoubleBuffered=true;TabStop=false;Cursor=Cursors.Hand;}
  protected override void OnPaint(PaintEventArgs e){e.Graphics.Clear(owner.BackColor);if(owner.Limit==0)return;int h=Math.Max(30,Height*Height/owner.content.Height),y=(Height-h)*owner.offset/owner.Limit;using(var b=new SolidBrush(StudioColors.Track))e.Graphics.FillRectangle(b,5,y,4,h);}
  protected override void OnMouseDown(MouseEventArgs e){if(e.Button==MouseButtons.Left){held=true;Capture=true;owner.Track(e.Y);}base.OnMouseDown(e);}
  protected override void OnMouseMove(MouseEventArgs e){if(held)owner.Track(e.Y);base.OnMouseMove(e);}
  protected override void OnMouseUp(MouseEventArgs e){held=false;Capture=false;base.OnMouseUp(e);}
  protected override void OnMouseWheel(MouseEventArgs e){owner.ScrollTo(owner.offset-e.Delta/3);base.OnMouseWheel(e);}
 }
 public NightScroll() { DoubleBuffered = true; Dock = DockStyle.Fill; TabStop = true;rail=new ScrollRail(this){Visible=false};Controls.Add(rail); }
 public void SetContent(Control c) { content = c; Hook(c); c.SizeChanged += delegate { Arrange(); }; Arrange(); }
 void Hook(Control c) { c.MouseWheel += delegate(object s, MouseEventArgs e) { ScrollTo(offset-e.Delta/3); };if(c.CanSelect)c.Enter+=delegate{if(!c.IsHandleCreated||!IsHandleCreated)return;Point point=PointToClient(c.PointToScreen(Point.Empty));if(point.Y<0)ScrollTo(offset+point.Y-12);else if(point.Y+c.Height>ClientSize.Height)ScrollTo(offset+point.Y+c.Height-ClientSize.Height+12);};foreach (Control child in c.Controls) Hook(child); }
 int Limit { get { return content == null ? 0 : Math.Max(0,content.Height-ClientSize.Height); } }
 void Arrange() { offset = Math.Min(offset,Limit); if (content != null) content.Location = new Point(0,-offset);if(rail!=null){rail.SetBounds(Math.Max(0,ClientSize.Width-14),0,14,ClientSize.Height);rail.Visible=Limit>0;rail.BringToFront();rail.Invalidate();}Invalidate(); }
 public void ScrollTo(int value) { offset = Math.Max(0,Math.Min(Limit,value)); Arrange(); }
 protected override void OnResize(EventArgs e) { base.OnResize(e); Arrange(); }
 protected override void OnMouseWheel(MouseEventArgs e) { ScrollTo(offset-e.Delta/3); base.OnMouseWheel(e); }
 void Track(int y) { ScrollTo((int)((double)Math.Max(0,y)/Math.Max(1,ClientSize.Height)*Limit)); }
 protected override void OnMouseDown(MouseEventArgs e) { base.OnMouseDown(e); if(e.X>=ClientSize.Width-20) { dragging=true; Capture=true; Track(e.Y); } }
 protected override void OnMouseMove(MouseEventArgs e) { base.OnMouseMove(e); if(dragging) Track(e.Y); }
 protected override void OnMouseUp(MouseEventArgs e) { base.OnMouseUp(e); dragging=false; Capture=false; }
 protected override void OnKeyDown(KeyEventArgs e) { if(e.KeyCode==Keys.PageDown) ScrollTo(offset+ClientSize.Height-40); else if(e.KeyCode==Keys.PageUp) ScrollTo(offset-ClientSize.Height+40); else if(e.KeyCode==Keys.Home) ScrollTo(0); else if(e.KeyCode==Keys.End) ScrollTo(Limit); base.OnKeyDown(e); }
}
public static class NotificationChime {
 [DllImport("winmm.dll", CharSet = CharSet.Unicode)] static extern bool PlaySound(string name, IntPtr module, uint flags);
 public static bool Play() { return PlaySound("SystemNotification",IntPtr.Zero,0x10001); }
 public static void Stop() { PlaySound(null,IntPtr.Zero,0); }
}

public partial class Settings {
 public string VoiceProvider = "System";
 public bool Swear = false;
 public bool Voice = true;
 public bool NotificationSound = true;
 public bool Paused = false;
 public bool ExerciseEnabled = true;
 public string LastMeal = "";
 public string LastBreak = "";
 public DateTime? ExerciseUtc;
 public List<string> History = new List<string>();
}
public static class Schedule {
 public static bool IsWorkday(DateTime utc) { var day=Local(utc).DayOfWeek;return day!=DayOfWeek.Saturday && day!=DayOfWeek.Sunday; }
 public static string Meal(DateTime utc, string previous) {
  DateTime d = Local(utc);
  if (d.DayOfWeek == DayOfWeek.Saturday || d.DayOfWeek == DayOfWeek.Sunday) return null;
  string kind = d.Hour == 10 && d.Minute < 5 ? "breakfast" : d.Hour == 14 && d.Minute >= 30 && d.Minute < 35 ? "lunch" : d.Hour == 18 && d.Minute >= 30 && d.Minute < 35 ? "dinner" : null;
  return kind == null || previous == d.ToString("yyyy-MM-dd") + "-" + kind ? null : kind;
 }
 public static string MealText(string kind) {
  if (kind == "breakfast") return "Пора поесть. 🍰⊹˚₊ Отложи работу и спокойно позавтракай";
  if (kind == "lunch") return "Время обеда. 🍜⊹˚₊ Сделай паузу и спокойно поешь вдали от компьютера";
  return "Пора поужинать. 🥞⊹˚₊ Отложи дела и удели время себе и еде";
 }
 public static bool ExerciseDue(Settings s, DateTime utc) { return IsWorkday(utc) && s.ExerciseEnabled && s.ExerciseUtc.HasValue && utc >= s.ExerciseUtc.Value && utc - s.ExerciseUtc.Value <= TimeSpan.FromHours(4); }
 public static void SetExercise(Settings s, bool enabled) { s.ExerciseEnabled = enabled; if (!enabled) s.ExerciseUtc = null; }
 public static readonly TimeZoneInfo Zone = TimeZoneInfo.FindSystemTimeZoneById("Central Europe Standard Time");
 public static DateTime Local(DateTime utc) { return TimeZoneInfo.ConvertTimeFromUtc(DateTime.SpecifyKind(utc, DateTimeKind.Utc), Zone); }
 public static string Due(DateTime utc, string previous) {
  DateTime d = Local(utc);
  if (d.DayOfWeek == DayOfWeek.Saturday || d.DayOfWeek == DayOfWeek.Sunday || d.Hour < 10 || d.Hour > 19 || d.Minute >= 5) return null;
  string id = d.ToString("yyyy-MM-dd-HH"); return id == previous ? null : id;
 }
 public static DateTime Next(DateTime utc) {
  DateTime d = Local(utc);
  DateTime n = d.Date.AddHours(d.Hour + 1);
  while (n.DayOfWeek == DayOfWeek.Saturday || n.DayOfWeek == DayOfWeek.Sunday || n.Hour < 10 || n.Hour > 19) n = n.AddHours(1);
  return n;
 }
}
public partial class MainForm : NightForm {
 static string dataFolder;
 static string Folder {get{return dataFolder??(dataFolder=ResolveFolder());}set{dataFolder=value;}}
 static readonly JavaScriptSerializer Json = new JavaScriptSerializer();
 string config = Path.Combine(Folder, "settings.json");
 Settings settings;
 NotifyIcon tray;
 System.Windows.Forms.Timer timer;
 Label clockLabel, nextLabel, breakfastLabel, status;
 Label schedulePeriod,scheduleMeals,scheduleWeekdays,exerciseHint;
 TextBox lastText;
 CheckBox voice, sound, pause, exerciseToggle;
 bool advancedVoiceBaseline;
 Button breakfastButton;
 FlowLayoutPanel homePage, settingsPage;
 NightTabs tabs;
 Color ink{get{return StudioColors.Ink;}} Color muted{get{return StudioColors.Muted;}} Color paper{get{return StudioColors.Surface;}} Color accent{get{return StudioColors.Accent;}}
 bool exiting;
 SoundPlayer player;
 MemoryStream audioStream;
 int audioVersion;
 public MainForm() {
  Directory.CreateDirectory(Folder);
  MigrateSettings();
  settings = new Settings();
  string loadWarning = "";
  if (File.Exists(config)) try {
   string raw=File.ReadAllText(config);legacyDesktopSettings=raw.IndexOf("\"SchemaVersion\"",StringComparison.Ordinal)<0;
   settings=Json.Deserialize<Settings>(raw);if(settings==null)throw new InvalidDataException(AppText.T("Настройки пусты","Settings are empty"));
   if(legacyDesktopSettings){
    File.Copy(config,config+".before-v2",true);settings.TimeZoneId="Central Europe Standard Time";
    if(raw.IndexOf("\"SpeechLanguage\"",StringComparison.Ordinal)<0)settings.SpeechLanguage="ru";
    settings.TextGeneration=settings.VoiceProvider!="System";settings.SetupComplete=true;
   }
  }catch(Exception error){
   if(Environment.GetCommandLineArgs().Contains("--health-check"))throw new InvalidDataException(AppText.T("Не удалось перенести существующие настройки; обновление должно быть отменено","Existing settings could not be migrated; update must roll back"),error);
   File.Copy(config,config+".unreadable-"+Guid.NewGuid().ToString("N"),false);settings=new Settings();legacyDesktopSettings=false;
   loadWarning=L.T("Не удалось прочитать настройки. Их исходная копия сохранена рядом с settings.json.","Could not read settings. The original copy was kept next to settings.json.");
  }
  if (settings == null) settings = new Settings();
  if(!TestMode&&!SharedStorage.IsIsolated(Environment.GetCommandLineArgs()))settings.RunAtLogin=StartupIntegration.IsEnabled(StartupIntegration.Folder);
  if(!TestMode&&!SharedStorage.IsIsolated(Environment.GetCommandLineArgs())){LegacyPersonalization.ImportCaptured(Path.Combine(Application.StartupPath,"user-legacy"),Folder);if(!settings.PersonalizationMigrated){var imported=LegacyPersonalization.Capture(Application.StartupPath,Folder);if(imported.ContainsKey("voice"))settings.VoiceProfile=imported["voice"];if(imported.ContainsKey("character"))settings.CharacterFile=imported["character"];settings.PersonalizationMigrated=true;}}
  if (!settings.LocalAiMigrated) { settings.LocalAiMigrated = true; }
  if (settings.History == null) settings.History = new List<string>();
  L.Language=settings.UiLanguage;StudioColors.SetMode(settings.ThemeMode);
  Text = L.T("Кавычка · маленькие ритуалы","Kavychka · little rituals"); ClientSize = new Size(760, Math.Min(980, Screen.PrimaryScreen.WorkingArea.Height - 60)); MinimumSize = new Size(760, 650);
  StartPosition = FormStartPosition.CenterScreen; BackColor = StudioColors.Background; Font = new Font("Segoe UI", 10); ForeColor = ink;
  Icon = SystemIcons.Information; AutoScaleMode = AutoScaleMode.Dpi;
  var stack = new FlowLayoutPanel { Dock = DockStyle.Fill, FlowDirection = FlowDirection.TopDown, WrapContents = false, AutoScroll = true, Padding = new Padding(28,24,20,20) };
  Controls.Add(stack);
  AddLabel(stack, L.T("М А Л Е Н Ь К И Е   Р И Т У А Л Ы","L I T T L E   R I T U A L S"), 9, 26).ForeColor = muted;
  var title = AddLabel(stack, L.T("Кавычка","Kavychka"), 34, 64); title.Font = new Font("Georgia", 34);
  AddLabel(stack, L.T("Немного тишины между большими делами.","A quiet moment between bigger things."), 11, 30).ForeColor = muted;
  tabs = new NightTabs { Width = 700, Height = Math.Max(350,ClientSize.Height-310), Margin = new Padding(0,8,0,10), Font = Font };
  var dayTab = new TabPage(L.T("Мой день","My day")) { BackColor = BackColor, Padding = new Padding(0,12,0,0) };
  var optionsTab = new TabPage(L.T("Настройки","Settings")) { BackColor = BackColor, Padding = new Padding(0,12,0,0) };
  tabs.TabPages.Add(dayTab); tabs.TabPages.Add(optionsTab); stack.Controls.Add(tabs);
  var dayScroll = new NightScroll(); var optionsScroll = new NightScroll(); dayTab.Controls.Add(dayScroll); optionsTab.Controls.Add(optionsScroll);
  homePage = Page(dayScroll); settingsPage = Page(optionsScroll);
  ClientSizeChanged += delegate { tabs.Height = Math.Max(350,ClientSize.Height-310); };
  var scheduleCard = Card(homePage);
  AddLabel(scheduleCard, L.T("01  /  РИТМ ДНЯ","01  /  DAILY RHYTHM"), 9, 27).ForeColor = muted;
  schedulePeriod=AddLabel(scheduleCard,"",14,35);
  scheduleMeals=AddLabel(scheduleCard,"",11,30);
  scheduleWeekdays=AddLabel(scheduleCard,"",9,25);scheduleWeekdays.ForeColor=muted;
  nextLabel = AddLabel(scheduleCard, "", 10, 28);
  clockLabel = AddLabel(scheduleCard, "", 9, 23); clockLabel.ForeColor = muted;
  var exerciseCard = Card(homePage);
  AddLabel(exerciseCard, L.T("02  /  ДВИЖЕНИЕ В СВОЁМ ТЕМПЕ","02  /  MOVE AT YOUR OWN PACE"), 9, 27).ForeColor = muted;
  exerciseToggle = Check(exerciseCard, L.T("Напоминать о зарядке после завтрака","Remind me to stretch after breakfast"), settings.ExerciseEnabled);
  exerciseToggle.CheckedChanged += delegate {if(refreshingAdvanced)return;SetDesktopExerciseEnabled(exerciseToggle.Checked);};
  breakfastLabel = AddLabel(exerciseCard, "", 10, 30);
  var row = Row(exerciseCard);
  breakfastButton = Button(row, L.T("Я позавтракала","I have had breakfast"), delegate { MarkBreakfast(); });
  Button(row, L.T("Отменить на сегодня","Cancel for today"), delegate { settings.ExerciseUtc = null; Save(); UpdateLabels(); });
  exerciseHint=AddLabel(exerciseCard,"",9,44);exerciseHint.ForeColor=muted;
  var messageCard = Card(homePage);
  AddLabel(messageCard, L.T("03  /  ЗАПИСКА ДЛЯ ТЕБЯ","03  /  A NOTE FOR YOU"), 9, 27).ForeColor = muted;
  lastText = new TextBox { Width = 595, Height = 65, Multiline = true, ReadOnly = true, BorderStyle = BorderStyle.None, BackColor = paper, ForeColor = ink, Font = new Font("Georgia", 12), ScrollBars = ScrollBars.None, Text = L.T("Начальство подождёт. Ты — нет.\r\nЗдесь появится твоё следующее напоминание.","Work can wait. You matter.\r\nYour next reminder will appear here.") }; messageCard.Controls.Add(lastText);
  var homeActions = Row(homePage);
  pause = Check(homeActions, L.T("Пауза всех напоминаний","Pause all reminders"), settings.Paused); pause.Width = 300;
  pause.CheckedChanged += delegate {if(refreshingAdvanced)return;if(pause.Checked)SetDesktopPause(-1);else{settings.Paused=false;settings.PauseUntilUtc=null;Save();}StopAudio();UpdateLabels();};
  Button(homeActions, L.T("Стоп голос","Stop voice"), delegate { StopAudio(); });
  Button(homeActions, L.T("В трей","Hide to tray"), delegate { Hide(); });
  voice = Check(new Panel(),L.T("Озвучивать напоминания","Read reminders aloud"),settings.Voice);
  advancedVoiceBaseline=settings.Voice;
  sound = Check(new Panel(),L.T("Системный звук при появлении уведомления","Play a chime when a reminder appears"),settings.NotificationSound);
  sound.CheckedChanged += delegate {if(refreshingAdvanced)return;settings.NotificationSound=sound.Checked;Save();};
  var startupCard = Card(settingsPage);
  AddLabel(startupCard, L.T("Всегда рядом","Always nearby"), 18, 40).Font = new Font("Georgia", 18);
  var bottom = Row(startupCard);
  Button(bottom, L.T("Автозапуск: вкл.","Enable startup"), delegate { SetStartup(true); });
  Button(bottom, L.T("Автозапуск: выкл.","Disable startup"), delegate { SetStartup(false); });
  AddLabel(startupCard, L.T("Крестик сворачивает в трей. Выход — через меню значка рядом с часами.","Closing this window hides it to the tray. Use the tray menu to exit."), 9, 40).ForeColor = muted;
  status = AddLabel(stack, loadWarning.Length > 0 ? loadWarning : L.T("Всё в своём темпе. Настройки голоса — на соседней вкладке.","At your own pace. Voice options are on the Settings tab."), 9, 90); status.ForeColor = muted;
  // Local generation never imports or uses credentials from earlier versions.
  var localVoiceCard = Card(settingsPage); settingsPage.Controls.SetChildIndex(localVoiceCard,0);
  AddLabel(localVoiceCard,L.T("Голос и уведомления","Voice and notifications"),21,45).Font=new Font("Georgia",21);
  AddLabel(localVoiceCard,L.T("Бесплатная локальная генерация. Интернет нужен только для установки компонентов.","Free local generation. Internet is only needed to install components."),10,46).ForeColor=muted;
  localVoiceCard.Controls.Add(voice); localVoiceCard.Controls.Add(sound);
  var localTests=Row(localVoiceCard);
  Button(localTests,L.T("Сохранить","Save"),delegate { SaveInputs(); });
  Button(localTests,L.T("Проверить напоминание","Test reminder"),async delegate { if(SaveInputs()) await StartDesktopReminder("break",false); });
  Button(localTests,L.T("Стоп голос","Stop voice"),delegate { StopAudio(); });
  InitializeLocalAi();
  dayScroll.SetContent(homePage); optionsScroll.SetContent(settingsPage);
  Save();
  tray = new NotifyIcon { Icon = SystemIcons.Information, Text = L.T("Кавычка · напоминания о перерывах","Kavychka · break reminders"), Visible = !TestMode };
  var menu = new ContextMenuStrip { BackColor = paper, ForeColor = ink, Renderer = new ToolStripProfessionalRenderer(new NightMenuColors()) };
  StudioColors.PrepareMenu(menu);
  menu.Opening+=delegate{L.Language=settings.UiLanguage;foreach(ToolStripItem item in menu.Items)item.Text=AppText.Caption(item.Text);tray.Text=L.T("Кавычка · напоминания о перерывах","Kavychka · break reminders");};
  menu.Items.Add(L.T("Открыть","Open"), null, delegate { OpenSimpleSettings(); });
  menu.Items.Add(L.T("Я позавтракала","I have had breakfast"), null, delegate { MarkBreakfast(); });
  menu.Items.Add(L.T("Зарядка: вкл. / выкл.","Enable / disable stretching"), null, delegate {SetDesktopExerciseEnabled(!settings.ExerciseEnabled);});
  menu.Items.Add(L.T("Пауза / продолжить","Pause / resume"), null, delegate {if(settings.Paused){settings.Paused=false;settings.PauseUntilUtc=null;Save();}else SetDesktopPause(-1);});
  menu.Items.Add(L.T("Выход","Exit"), null, delegate { exiting = true; Close(); });
  tray.ContextMenuStrip = menu; tray.DoubleClick += delegate { OpenSimpleSettings(); };
  if(!TestMode){Opacity=0;ShowInTaskbar=false;Shown+=delegate{Hide();Opacity=1;ShowInTaskbar=true;};}
  FormClosing += delegate(object sender, FormClosingEventArgs e) { if (!exiting && e.CloseReason == CloseReason.UserClosing) { e.Cancel = true; Hide(); } };
  FormClosed += delegate { timer.Stop(); tray.Dispose(); StopAudio(); CloseLocalAi();  };
  InitializeDesktop();timer = new System.Windows.Forms.Timer { Interval = 1000 }; timer.Tick += async delegate { await Tick(); }; timer.Start(); UpdateLabels();
 }
 FlowLayoutPanel Page(Control c) { var p = new FlowLayoutPanel { Width = 660, MinimumSize = new Size(660,0), MaximumSize = new Size(660,0), AutoSize = true, AutoSizeMode = AutoSizeMode.GrowAndShrink, FlowDirection = FlowDirection.TopDown, WrapContents = false, Margin = Padding.Empty }; c.Controls.Add(p); return p; }
 FlowLayoutPanel Card(Control c) { var p = new FlowLayoutPanel { Width = 650, MinimumSize = new Size(650,0), MaximumSize = new Size(650,0), AutoSize = true, AutoSizeMode = AutoSizeMode.GrowAndShrink, FlowDirection = FlowDirection.TopDown, WrapContents = false, BackColor = paper, Padding = new Padding(22,18,18,15), Margin = new Padding(0,0,0,14) }; c.Controls.Add(p); return p; }
 Label AddLabel(Control c, string text, float size, int height) { var l = new Label { Text = text, Width = 595, Height = height, Font = new Font("Segoe UI", size), ForeColor = ink, Margin = new Padding(0,0,0,2) }; c.Controls.Add(l); return l; }
 FlowLayoutPanel Row(Control c) { var p = new FlowLayoutPanel { Width = 605, Height = 48, WrapContents = false, Margin = new Padding(0,5,0,3) }; c.Controls.Add(p); return p; }
 Button Button(Control c, string text, EventHandler handler) { var b = new Button { Text = text, AutoSize = true, Height = 36, FlatStyle = FlatStyle.Flat, BackColor = StudioColors.Soft, ForeColor = ink, Padding = new Padding(10,4,10,4), Cursor = Cursors.Hand, Margin = new Padding(0,2,8,2) }; b.FlatAppearance.BorderSize = 0; b.FlatAppearance.MouseOverBackColor = StudioColors.Hover; b.Click += handler; c.Controls.Add(b); return b; }
 CheckBox Check(Control c, string text, bool value) { var b = new CheckBox { Text = text, Checked = value, Width = 595, Height = 32, ForeColor = ink, Cursor = Cursors.Hand, FlatStyle = FlatStyle.Flat, Margin = new Padding(0,2,0,2) }; c.Controls.Add(b); return b; }
 void MarkBreakfast() {DesktopBreakfast();}
 void Restore() {L.Language=settings.UiLanguage;RefreshAdvancedLocalization();refreshingAdvanced=true;try{if(voice.Checked==advancedVoiceBaseline)voice.Checked=settings.Voice;advancedVoiceBaseline=settings.Voice;sound.Checked=settings.NotificationSound;pause.Checked=settings.Paused;exerciseToggle.Checked=settings.ExerciseEnabled;}finally{refreshingAdvanced=false;}Show();WindowState=FormWindowState.Normal;Activate();}
 string settingsSaveError="";
 bool Save() {
  try {SettingsFile.Write(config,Json.Serialize(settings));settingsSaveError="";return true;}
  catch(Exception error){settingsSaveError=L.T("Не удалось сохранить настройки на диск. Проверь место и доступ к папке: ","Could not save settings. Check free space and folder access: ")+error.Message;if(status!=null)status.Text=settingsSaveError;preparationStatus=settingsSaveError;LogDesktop(error);return false;}
 }
 bool SaveInputs() {
  bool oldSwear=settings.Swear,oldVoice=settings.Voice;settings.Swear=localSwear.Checked;settings.Voice=voice.Checked;
  if(!Save()){settings.Swear=oldSwear;settings.Voice=oldVoice;return false;}advancedVoiceBaseline=settings.Voice;status.Text=L.T("Настройки сохранены.","Settings saved.");return true;
 }
 void SetStartup(bool enabled) {
  try {
   bool previous=settings.RunAtLogin;ApplyStartupPreference(enabled);settings.RunAtLogin=enabled;if(!Save()){settings.RunAtLogin=previous;ApplyStartupPreference(previous);return;}
   status.Text = enabled ? L.T("Автозапуск включён. Не перемещай папку приложения.","Startup enabled. Keep the application folder in this location.") : L.T("Автозапуск выключен.","Startup disabled.");
  } catch(Exception error) {LogDesktop(error);status.Text=L.T("Не удалось изменить автозапуск: ","Could not change startup: ")+error.Message;}
 }
 void ApplyStartupPreference(bool enabled){if(TestMode||SharedStorage.IsIsolated(Environment.GetCommandLineArgs()))throw new InvalidOperationException(L.T("Автозапуск изменяется только при обычном запуске приложения.","Startup can only be changed during a regular application launch."));StartupIntegration.SetEnabled(StartupIntegration.Folder,Application.ExecutablePath,enabled);}
 void UpdateLabels() {
  var now = DateTime.UtcNow;
  if(schedulePeriod!=null){Func<int,string> time=minute=>DateTime.Today.AddMinutes(minute).ToString("HH:mm");schedulePeriod.Text=L.T("Перерывы каждые ","Breaks every ")+settings.BreakEvery+L.T(" мин · "," min · ")+time(settings.WorkStart)+"–"+time(settings.WorkEnd);scheduleMeals.Text=settings.MealsEnabled?L.Category("breakfast")+" "+time(settings.BreakfastTime)+" / "+L.Category("lunch")+" "+time(settings.LunchTime)+" / "+L.Category("dinner")+" "+time(settings.DinnerTime):L.T("Напоминания о еде выключены","Meal reminders are off");string[] days=L.Language=="ru"?new[]{"Вс","Пн","Вт","Ср","Чт","Пт","Сб"}:new[]{"Sun","Mon","Tue","Wed","Thu","Fri","Sat"};scheduleWeekdays.Text=settings.WorkDays.Length==0?L.T("Рабочие дни не выбраны","No workdays selected"):string.Join(", ",settings.WorkDays.Select(day=>days[day]));exerciseHint.Text=L.T("Через ","After ")+settings.ExerciseAfter+L.T(" мин после отметки завтрака. Еда и перерывы работают отдельно."," min from marking breakfast. Meals and breaks are independent.");}
  clockLabel.Text = RitualSchedule.Local(settings,now).ToString("dd.MM.yyyy HH:mm:ss")+" · "+RitualSchedule.Zone(settings).DisplayName;
  nextLabel.Text = settings.Paused ? L.T("Напоминания на паузе","Reminders paused") : L.T("Расписание и проверки активности: настройки у маскота","Schedule and activity detection: open mascot settings");
  breakfastButton.Enabled = settings.ExerciseEnabled;
  breakfastLabel.Text = !settings.ExerciseEnabled ? L.T("Разминка выключена","Stretch timer disabled") : settings.ExerciseUtc.HasValue ? L.T("Разминка: ","Stretch: ") + RitualSchedule.Local(settings,settings.ExerciseUtc.Value).ToString("dd.MM HH:mm") : L.T("Отметь завтрак, чтобы запустить таймер разминки — в любой день.","Mark breakfast to start the stretch timer — any day.");
 }
 async Task Tick() {
  UpdateLabels();await DesktopTick();
 }
 async Task Remind(bool exercise,string meal=null,bool automatic=false) {await StartDesktopReminder(meal??(exercise?"exercise":"break"),automatic);}
 Form ShowReminder(string text, bool exercise, string meal = null) {
  var popup = new NightForm { Text = L.T("Кавычка · маленькие ритуалы","Kavychka · little rituals"), ClientSize = new Size(500,325), TopMost = true, StartPosition = FormStartPosition.Manual, BackColor = paper, Font = Font, ForeColor = ink, FormBorderStyle = FormBorderStyle.FixedDialog, MaximizeBox = false, MinimizeBox = false };
  var area = Screen.PrimaryScreen.WorkingArea; popup.Location = new Point(area.Right - popup.Width - 20, area.Bottom - popup.Height - 20);
  var heading = new Label { Text = meal != null ? (meal == "breakfast" ? L.T("Время завтрака","Breakfast time") : meal == "lunch" ? L.T("Время обеда","Lunch time") : L.T("Время ужина","Dinner time")) : exercise ? L.T("Немного движения","A little movement") : L.T("Время выдохнуть","Time for a breather"), Font = new Font("Georgia",21), Location = new Point(24,60), Size = new Size(450,40), ForeColor = accent }; popup.Controls.Add(heading);
  var body = new TextBox { Text = text, ReadOnly = true, Multiline = true, BorderStyle = BorderStyle.None, BackColor = paper, ForeColor = ink, Font = new Font("Segoe UI", 12), Location = new Point(24,115), Size = new Size(450,125), ScrollBars = ScrollBars.None, TabStop = false };
  popup.Controls.Add(body);
  var buttons = new FlowLayoutPanel { Location = new Point(24,260), Size = new Size(450,50) }; popup.Controls.Add(buttons);
  Button(buttons, meal == null ? L.T("Всё, встаю!","I am getting up!") : L.T("Иду кушать","I am going to eat"), delegate { StopAudio(); popup.Close(); });
  Button(buttons, L.T("Стоп голос","Stop voice"), delegate { StopAudio(); });
  popup.FormClosed += delegate { StopAudio(); }; popup.Shown += delegate { body.Select(0,0); buttons.Controls[0].Focus(); }; popup.Show();
  var closeTimer = new System.Windows.Forms.Timer { Interval = 120000 }; closeTimer.Tick += delegate { closeTimer.Stop(); popup.Close(); closeTimer.Dispose(); }; popup.FormClosed += delegate { closeTimer.Dispose(); }; closeTimer.Start();
  return popup;
 }
 void StopAudio() { audioVersion++;activeSystemPrompt=null; NotificationChime.Stop();if(systemSpeech!=null)systemSpeech.SpeakAsyncCancelAll();if(pet!=null)pet.Speaking=false; if (player != null) { player.Stop(); player.Dispose(); player = null; } if (audioStream != null) { audioStream.Dispose(); audioStream = null; } }
 [STAThread] public static void Main(string[] args) {
  ServicePointManager.SecurityProtocol = SecurityProtocolType.Tls12;
  try { MainCore(args); } catch(Exception e) {Environment.ExitCode=1;string path=args.Contains("--preview")&&args.Length>1?args[1]+".error.txt":Path.Combine(Application.StartupPath,"startup-error.log");File.WriteAllText(path,e.ToString());if(!args.Any(a=>new[]{"--preview","--v2-preview","--setup-smoke","--self-test","--health-check","--capture-legacy"}.Contains(a)))MessageBox.Show(e.GetBaseException().Message,L.T("Кавычка","Kavychka"),MessageBoxButtons.OK,MessageBoxIcon.Error);}
 }
 static void MainCore(string[] args) {
  if(args.Contains("--health-check")){int at=Array.IndexOf(args,"--health-check");CheckAppHealth(args[at+1]);return;}
  if(args.Contains("--capture-legacy")){int at=Array.IndexOf(args,"--capture-legacy");LegacyPersonalization.Capture(args[at+1],Folder);return;}
  if(args.Contains("--v2-preview")){
   Application.EnableVisualStyles();Application.SetCompatibleTextRenderingDefault(false);
   int at=Array.IndexOf(args,"--v2-preview");L.Language=args.Contains("--en")?"en":"ru";int theme=Array.IndexOf(args,"--theme");var settings=new Settings{UiLanguage=L.Language,SpeechLanguage=L.Language,ThemeMode=theme<0?"system":args[theme+1]};StudioColors.SetMode(settings.ThemeMode);
   int presetKind=Array.IndexOf(args,"--preset-kind");using(var target=args.Contains("--pet")?(Form)new PetForm():presetKind>=0?new PresetEditorForm(Folder,Application.StartupPath,args[presetKind+1],""):(Form)new SimpleSettingsForm(settings,new List<Dictionary<string,object>>(),true)){
    if(args.Contains("--paused")){var pet=(PetForm)target;pet.Paused=true;pet.PauseText="15 min";}
    target.Show();if(target is SimpleSettingsForm){var simple=(SimpleSettingsForm)target;int page=Array.IndexOf(args,"--page");if(page>=0)simple.SelectPage(int.Parse(args[page+1]));var upcoming=RitualSchedule.Next(settings,DateTime.UtcNow);simple.UpdateStatus(upcoming==null?L.T("Нет запланированных напоминаний","No reminders scheduled"):RitualSchedule.Local(settings,upcoming.Utc).ToString("dd.MM · HH:mm")+"  ·  "+L.Category(upcoming.Category),L.T("Базовые напоминания готовы. Модели подключаются по желанию.","Basic reminders are ready. Models are optional."),false,0,false,Folder);}
    target.Refresh();using(var bitmap=target is PetForm?((PetForm)target).RenderSnapshot():new Bitmap(target.Width,target.Height)){if(!(target is PetForm))target.DrawToBitmap(bitmap,new Rectangle(0,0,bitmap.Width,bitmap.Height));bitmap.Save(args[at+1]);}target.Close();
   }return;
  }
  if (args.Contains("--setup-smoke")) {
   Application.EnableVisualStyles();Application.SetCompatibleTextRenderingDefault(false);var f=new MainForm();
   f.Shown+=async delegate {try {
    await f.EnsureBackend();if(args.Contains("--install-runtime"))await f.InstallRuntime(true);
    var result=await f.Api("/status");result["data_directory"]=Folder;
    if(args.Contains("--settings-check")){await f.LoadLocalSettings();result["character_preset"]=f.settings.CharacterFile;result["character_description"]=PersonalPresets.Description(Folder,f.settings.CharacterFile);result["personalization_migrated"]=f.settings.BackendPersonalizationMigrated;}
    if(args.Contains("--library-check")) {
     await f.LoadLocalSettings();await f.RefreshLibrary();if(f.library.Items.Count==0)throw new Exception(AppText.T("Библиотека пуста","Library is empty"));
     var first=D(f.library.Items[0].Tag);byte[] wav=await f.ApiBytes("/audio/"+S(first,"id"));using(var stream=new MemoryStream(wav))using(var testPlayer=new SoundPlayer(stream)){testPlayer.Load();if(!testPlayer.IsLoadCompleted)throw new Exception(AppText.T("Windows не смогла загрузить WAV","Windows WAV load failed"));}
     f.library.Items[0].Selected=true;f.tabs.SelectedIndex=1;((NightScroll)f.tabs.SelectedTab.Controls[0]).ScrollTo(int.MaxValue);f.Refresh();
     using(var bitmap=new Bitmap(f.Width,f.Height)){f.DrawToBitmap(bitmap,new Rectangle(0,0,bitmap.Width,bitmap.Height));bitmap.Save(args[1]+".png");}
     result["library_count"]=f.library.Items.Count;result["windows_wav_loaded"]=true;
    }
    File.WriteAllText(args[1],Json.Serialize(result));
   } catch(Exception e){File.WriteAllText(args[1],"FAIL "+e.ToString());}finally {f.exiting=true;f.Close();}};
   Application.Run(f);return;
  }
  if (args.Contains("--sound-test")) { bool ok = NotificationChime.Play(); Thread.Sleep(1400); NotificationChime.Stop(); File.WriteAllText(args[1],ok ? "PASS Windows accepted SystemNotification playback" : "FAIL Windows rejected notification sound"); return; }
  if (args.Contains("--self-test")) { Tests.Run(args.Length > 1 ? args[1] : "test-results.txt"); return; }
  if (args.Contains("--preview")) {
   Application.EnableVisualStyles(); Application.SetCompatibleTextRenderingDefault(false);
   using (var preview = new MainForm()) { preview.timer.Stop(); if (args.Contains("--settings")) tabsSelect(preview); if (args.Contains("--library")) preview.tabs.SelectedIndex=1; if (args.Contains("--models")) preview.tabs.SelectedIndex=2; if (args.Contains("--character")) preview.tabs.SelectedIndex=3; preview.Show(); preview.Refresh(); Form target = preview;
    if (preview.tabs.TabPages.Count != 5 || preview.FormBorderStyle != FormBorderStyle.None) throw new Exception("Night UI verification failed: tabs="+preview.tabs.TabPages.Count+", border="+preview.FormBorderStyle);
    var scroll = (NightScroll)preview.tabs.SelectedTab.Controls[0]; var content = scroll.Content; scroll.ScrollTo(int.MaxValue); if (content.Height > scroll.Height && content.Top >= 0) throw new Exception(AppText.T("Ошибка проверки прокрутки","Scroll verification failed"));
    if (!args.Contains("--bottom")) scroll.ScrollTo(0);
    if (args.Contains("--popup")) target = preview.ShowReminder("Кавычка, начальство переживёт пять минут без твоих подвигов. Время перерыва!",false);
    target.Refresh(); using (var bitmap = new Bitmap(target.Width, target.Height)) { target.DrawToBitmap(bitmap, new Rectangle(0,0,bitmap.Width,bitmap.Height)); bitmap.Save(args[1]); } if (target != preview) target.Close(); preview.exiting = true; preview.Close(); } return;
  }
  bool fresh;
  using (var mutex = new Mutex(true, "Local\\KavychkaBreaks-v1", out fresh)) {
   if (!fresh) { NightForm.Notice(null,L.T("Кавычка уже работает. Откройте её значок рядом с часами.","Kavychka is already running. Open its icon in the system tray."),L.T("Кавычка","Kavychka")); return; }
   ServicePointManager.SecurityProtocol = SecurityProtocolType.Tls12;
   Application.EnableVisualStyles(); Application.SetCompatibleTextRenderingDefault(false);
   var form = new MainForm();
   if (args.Contains("--tray")) form.Shown += delegate { form.Hide(); };
   Application.Run(form);
  }
 }
 static void tabsSelect(MainForm f) { f.tabs.SelectedIndex = 4; }
}
public class NightMenuColors : ProfessionalColorTable {
 public override Color ToolStripDropDownBackground { get { return StudioColors.Surface; } }
 public override Color ImageMarginGradientBegin { get { return ToolStripDropDownBackground; } }
 public override Color ImageMarginGradientMiddle { get { return ToolStripDropDownBackground; } }
 public override Color ImageMarginGradientEnd { get { return ToolStripDropDownBackground; } }
 public override Color MenuItemSelected { get { return StudioColors.Soft; } }
 public override Color MenuItemBorder { get { return StudioColors.Border; } }
 public override Color MenuBorder { get { return StudioColors.Border; } }
 public override Color SeparatorDark { get { return StudioColors.Border; } }
 public override Color SeparatorLight { get { return StudioColors.Border; } }
}
public static class Tests {
 public static void Run(string output) {
  var log = new List<string>();
  Action<bool,string> check = (ok,name) => { if (!ok) throw new Exception(name); log.Add("PASS " + name); };
  Func<int,int,DateTime> monday = (h,m) => new DateTime(2026,10,19,h,m,0,DateTimeKind.Utc);
  try {
   check(Schedule.Due(monday(7,0), "") == null,"09:00 no reminder");
   check(Schedule.Due(monday(8,0), "") == "2026-10-19-10","10:00 first reminder");
   check(Schedule.Due(monday(17,0), "") != null,"19:00 included");
   check(Schedule.Due(monday(18,0), "") == null,"20:00 excluded");
   check(Schedule.Due(monday(8,0), "2026-10-19-10") == null,"restart deduplication");
   check(Schedule.Due(monday(8,4), "") != null,"short wake grace");
   check(Schedule.Due(monday(8,5), "") == null,"no old reminders backlog");
   check(Schedule.Due(new DateTime(2026,10,18,8,0,0,DateTimeKind.Utc), "") == null,"weekend excluded");
   check(Schedule.Local(new DateTime(2026,12,1,9,0,0,DateTimeKind.Utc)).Hour == 10,"winter UTC+1");
   check(Schedule.Local(new DateTime(2026,7,1,8,0,0,DateTimeKind.Utc)).Hour == 10,"summer UTC+2");
   check(Schedule.Next(new DateTime(2026,10,23,17,0,0,DateTimeKind.Utc)) == new DateTime(2026,10,26,10,0,0),"weekend and DST next reminder");
   var breakfast = monday(6,30); check(Schedule.Local(breakfast.AddHours(2)).Hour == 10,"two elapsed hours after breakfast");
   check(Schedule.Meal(monday(8,0), "") == "breakfast", "breakfast at 10:00 Budapest");
   check(Schedule.Meal(monday(12,30), "") == "lunch", "lunch at 14:30 Budapest");
   check(Schedule.Meal(monday(16,30), "") == "dinner", "dinner at 18:30 Budapest");
   check(Schedule.Meal(monday(12,29), "") == null, "lunch never early");
   check(Schedule.Meal(monday(12,35), "") == null, "no stale lunch backlog");
   check(Schedule.Meal(monday(12,30), "2026-10-19-lunch") == null, "meal restart deduplication");
   check(Schedule.Meal(monday(16,30), "2026-10-19-lunch") == "dinner", "lunch does not suppress dinner");
   check(Schedule.Meal(new DateTime(2026,10,18,12,30,0,DateTimeKind.Utc), "") == null, "no Sunday meal");
   check(Schedule.Meal(new DateTime(2026,10,17,8,0,0,DateTimeKind.Utc), "") == null, "no Saturday breakfast");
   check(Schedule.Meal(new DateTime(2026,12,1,13,30,0,DateTimeKind.Utc), "") == "lunch", "winter lunch UTC+1");
   var state = new Settings { ExerciseUtc = monday(8,0) };
   check(Schedule.ExerciseDue(state,monday(8,0)), "exercise enabled and due");
   Schedule.SetExercise(state,false);
   check(!state.ExerciseUtc.HasValue && !Schedule.ExerciseDue(state,monday(8,0)), "disabling exercise clears pending timer");
   check(Schedule.Due(monday(8,0),state.LastBreak) != null && Schedule.Meal(monday(8,0),state.LastMeal) == "breakfast", "exercise switch leaves break and meal active");
   Schedule.SetExercise(state,true);
   check(!Schedule.ExerciseDue(state,monday(8,0)), "reenabling requires new breakfast mark");
   state.ExerciseUtc = monday(10,0);
   check(!Schedule.ExerciseDue(state,monday(9,59)) && Schedule.ExerciseDue(state,monday(10,0)), "new breakfast timer fires exactly when due");
   check(!Schedule.ExerciseDue(state,monday(14,1)), "stale exercise ignored");
   foreach(var weekend in new[]{new DateTime(2026,10,17,10,0,0,DateTimeKind.Utc),new DateTime(2026,10,18,10,0,0,DateTimeKind.Utc)}) {
    var weekendState=new Settings {ExerciseUtc=weekend};
    check(!Schedule.IsWorkday(weekend) && !Schedule.ExerciseDue(weekendState,weekend),"weekend exercise suppressed "+weekend.DayOfWeek);
    foreach(int hour in new[]{8,12,16}) {
     var time=weekend.Date.AddHours(hour).AddMinutes(hour==8?0:30);
     check(Schedule.Due(time,"")==null && Schedule.Meal(time,"")==null,"weekend breaks and meals suppressed "+time.ToString("ddd HH:mm"));
    }
   }
   check(!Schedule.IsWorkday(new DateTime(2026,10,16,22,30,0,DateTimeKind.Utc)),"Friday UTC is already Saturday in Budapest");
   check(Schedule.IsWorkday(new DateTime(2026,10,18,22,30,0,DateTimeKind.Utc)),"Sunday UTC is already Monday in Budapest");
   var serializer = new JavaScriptSerializer();
   var migrated = serializer.Deserialize<Settings>("{\"Model\":\"custom\",\"LastBreak\":\"old\"}");
   check(migrated.ExerciseEnabled && migrated.LastBreak == "old", "old settings migrate without losing values");
   Schedule.SetExercise(state,false);
   check(!serializer.Deserialize<Settings>(serializer.Serialize(state)).ExerciseEnabled, "exercise preference persists");
   check(Schedule.MealText("lunch") == "Время обеда. 🍜⊹˚₊ Сделай паузу и спокойно поешь вдали от компьютера", "exact requested lunch text");
   check(!serializer.Serialize(migrated).Contains("Secret"),"legacy API secrets are not retained");
   check(migrated.NotificationSound,"notification sound enabled when upgrading old settings");
   var silent = new Settings { NotificationSound = false }; check(!serializer.Deserialize<Settings>(serializer.Serialize(silent)).NotificationSound,"notification sound preference persists");
   File.WriteAllLines(output,log); Environment.ExitCode = 0;
  } catch(Exception e) { log.Add("FAIL " + e.Message); File.WriteAllLines(output,log); Environment.ExitCode = 1; }
 }
}
