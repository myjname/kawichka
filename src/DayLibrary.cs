using System;
using System.Collections.Generic;
using System.Drawing;
using System.Linq;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
using System.Windows.Forms;

public sealed class DayEntry {
 public string Id,Text,Category,State;
 public bool Followup;
}

public sealed class DayLibraryForm:NightForm {
 [DllImport("user32.dll")]static extern bool ShowScrollBar(IntPtr window,int bar,bool show);
 readonly ListView list;
 readonly TextBox text;
 readonly Label feedback;
 readonly StudioToggle automatic,approve;
 readonly NumericUpDown count;
 readonly ComboBox category;
 readonly CheckBox followup;
 readonly List<Control> actions=new List<Control>();
 readonly Timer refresh=new Timer{Interval=3000};
 readonly ContextMenuStrip selectionMenu=new ContextMenuStrip();
 List<DayEntry> entries=new List<DayEntry>();
 bool busy,changingOptions,savedAuto,savedKeep,sizingColumns;
 public Func<Task<List<DayEntry>>> LoadEntries;
 public Func<string,Task> Listen;
 public Action Stop,Cancel;
 public Func<string> Progress;
 public Func<Task> Prepare;
 public Func<string[],string,Task> SelectEntries;
 public Func<int,string,bool,Task> Generate;
 public Func<string[],Task> Revoice,Export;
 public Func<bool,bool,Task> OptionsChanged;
 static readonly string[] Categories={"break","breakfast","lunch","dinner","exercise"};
 public DayLibraryForm(string mode,bool auto,bool keep,bool canRevoice=true,bool canCancel=true){
  savedAuto=auto;savedKeep=keep;
  Text=L.T("Фразы на сегодня","Today's phrases");ClientSize=new Size(860,790);MinimumSize=new Size(760,720);StartPosition=FormStartPosition.CenterParent;BackColor=StudioColors.Background;ForeColor=StudioColors.Ink;Font=new Font("Segoe UI",10);
  var body=new TableLayoutPanel{Dock=DockStyle.Fill,Padding=new Padding(24,18,24,12),ColumnCount=1,RowCount=9,BackColor=BackColor};Controls.Add(body);
  foreach(float height in new[]{40f,42f,48f,48f})body.RowStyles.Add(new RowStyle(SizeType.Absolute,height));body.RowStyles.Add(new RowStyle(SizeType.Percent,100));foreach(float height in new[]{76f,48f,62f,52f})body.RowStyles.Add(new RowStyle(SizeType.Absolute,height));
  body.Controls.Add(new Label{Text=L.T("Твой дневной список","Your daily list"),Dock=DockStyle.Fill,Font=new Font("Segoe UI",20,FontStyle.Bold)},0,0);
  body.Controls.Add(new Label{Text=mode,Dock=DockStyle.Fill,ForeColor=StudioColors.Muted},0,1);
  automatic=new StudioToggle{Text=L.T("Готовить новый день автоматически","Prepare each new day automatically"),Checked=auto,Dock=DockStyle.Fill};body.Controls.Add(automatic,0,2);
  approve=new StudioToggle{Text=L.T("Разрешать новые дневные фразы без ручного отбора","Allow new daily phrases without manual review"),Checked=keep,Dock=DockStyle.Fill};body.Controls.Add(approve,0,3);
  automatic.CheckedChanged+=async delegate{await ChangeOptions();};approve.CheckedChanged+=async delegate{await ChangeOptions();};
  list=new ListView{Dock=DockStyle.Fill,View=View.Details,FullRowSelect=true,MultiSelect=true,HideSelection=false,BackColor=StudioColors.Surface,ForeColor=ForeColor,BorderStyle=BorderStyle.None,OwnerDraw=true};
  list.Columns.Add(L.T("Категория","Category"),95);list.Columns.Add(L.T("Тип","Type"),105);list.Columns.Add(L.T("Состояние","Status"),155);list.Columns.Add(L.T("Фраза","Phrase"),430);body.Controls.Add(list,0,4);
  list.ClientSizeChanged+=delegate{FitColumns();};
  list.KeyDown+=delegate(object sender,KeyEventArgs e){if(e.Control&&e.KeyCode==Keys.A){foreach(ListViewItem item in list.Items)item.Selected=true;e.Handled=true;e.SuppressKeyPress=true;}};
  StudioColors.PrepareMenu(selectionMenu);selectionMenu.Items.Add(L.T("Выделить все · Ctrl+A","Select all · Ctrl+A"),null,delegate{foreach(ListViewItem item in list.Items)item.Selected=true;});selectionMenu.Items.Add(L.T("Снять выделение","Clear selection"),null,delegate{foreach(ListViewItem item in list.Items)item.Selected=false;});list.ContextMenuStrip=selectionMenu;
  list.DrawColumnHeader+=delegate(object sender,DrawListViewColumnHeaderEventArgs e){using(var brush=new SolidBrush(StudioColors.Surface))e.Graphics.FillRectangle(brush,e.Bounds);TextRenderer.DrawText(e.Graphics,e.Header.Text,Font,e.Bounds,StudioColors.Muted,TextFormatFlags.Left|TextFormatFlags.VerticalCenter);};
  list.DrawSubItem+=delegate(object sender,DrawListViewSubItemEventArgs e){using(var brush=new SolidBrush(e.Item.Selected?StudioColors.Soft:StudioColors.Surface))e.Graphics.FillRectangle(brush,e.Bounds);TextRenderer.DrawText(e.Graphics,e.SubItem.Text,Font,e.Bounds,StudioColors.Ink,TextFormatFlags.Left|TextFormatFlags.VerticalCenter|TextFormatFlags.EndEllipsis);};
  text=new TextBox{Dock=DockStyle.Fill,Multiline=true,ReadOnly=true,BackColor=StudioColors.Surface,ForeColor=ForeColor,BorderStyle=BorderStyle.None};body.Controls.Add(text,0,5);
  list.SelectedIndexChanged+=delegate{if(list.SelectedItems.Count>0)text.Text=((DayEntry)list.SelectedItems[0].Tag).Text;};list.DoubleClick+=async delegate{await Run(ListenSelected);};
  var tools=new FlowLayoutPanel{Dock=DockStyle.Fill,WrapContents=false};body.Controls.Add(tools,0,6);
  Button(tools,L.T("Слушать","Listen"),105,ListenSelected);var stop=new StudioButton{Text=L.T("Стоп","Stop"),Width=80,Height=38,Primary=false};tools.Controls.Add(stop);stop.Click+=delegate{if(Stop!=null)Stop();};
  Button(tools,L.T("Оставить","Keep"),100,()=>Selection("keep"));Button(tools,L.T("Исключить","Exclude"),110,()=>Selection("exclude"));Button(tools,L.T("Озвучить заново","Revoice"),155,async delegate{if(Revoice!=null)await Revoice(Ids(true));}).Visible=canRevoice;Button(tools,canRevoice?"ZIP":L.T("ZIP текстов","Text ZIP"),canRevoice?70:125,async delegate{if(Export!=null)await Export(Ids(false));});
  var batch=new FlowLayoutPanel{Dock=DockStyle.Fill,WrapContents=false,Padding=new Padding(0,8,0,0)};body.Controls.Add(batch,0,7);
  count=new NumericUpDown{Minimum=1,Maximum=100,Value=5,Width=48,BackColor=StudioColors.Surface,ForeColor=ForeColor};batch.Controls.Add(count);
  category=new StudioCombo{DropDownStyle=ComboBoxStyle.DropDownList,Width=100,BackColor=StudioColors.Surface,ForeColor=ForeColor};category.Items.AddRange(Categories.Select(L.Category).ToArray());category.SelectedIndex=0;batch.Controls.Add(category);
  followup=new StudioToggle{Text=L.T("Повторы","Follow-ups"),Width=120,Height=38};batch.Controls.Add(followup);
  Button(batch,L.T("Создать ещё","Create more"),119,async delegate{if(Generate!=null)await Generate((int)count.Value,Categories[category.SelectedIndex],followup.Checked);});
  Button(batch,L.T("Подготовить день","Prepare day"),165,async delegate{if(Prepare!=null)await Prepare();});var cancel=new StudioButton{Text=L.T("Отмена","Cancel"),Primary=false,Width=95,Height=38,Visible=canCancel};batch.Controls.Add(cancel);cancel.Click+=delegate{if(Cancel!=null)Cancel();};
  feedback=new Label{Dock=DockStyle.Fill,ForeColor=StudioColors.Muted};body.Controls.Add(feedback,0,8);
  refresh.Tick+=async delegate{if(!busy)await Run(Reload,false);else if(Progress!=null)feedback.Text=Progress();};Shown+=async delegate{StudioColors.NativeTheme(this);await Run(Reload,false);refresh.Start();};FormClosed+=delegate{if(Stop!=null)Stop();};
 }
 StudioButton Button(Control parent,string label,int width,Func<Task> action){var button=new StudioButton{Text=label,Primary=false,Width=width,Height=38};parent.Controls.Add(button);actions.Add(button);button.Click+=async delegate{await Run(action);};return button;}
 void FitColumns(){if(sizingColumns)return;sizingColumns=true;try{list.Columns[3].Width=Math.Max(160,list.ClientSize.Width-list.Columns.Cast<ColumnHeader>().Take(3).Sum(column=>column.Width)-1);if(list.IsHandleCreated&&list.Columns.Cast<ColumnHeader>().Sum(column=>column.Width)<=list.ClientSize.Width)ShowScrollBar(list.Handle,0,false);}finally{sizingColumns=false;}}
 string[] Ids(bool require){var selected=list.SelectedItems.Cast<ListViewItem>().Select(item=>((DayEntry)item.Tag).Id).ToArray();if(require&&selected.Length==0)throw new InvalidOperationException(L.T("Выбери фразы в списке.","Select phrases in the list."));return selected.Length>0?selected:entries.Select(item=>item.Id).ToArray();}
 async Task ListenSelected(){string[] ids=Ids(true);if(Listen!=null)await Listen(ids[0]);}
 async Task Selection(string action){if(SelectEntries!=null)await SelectEntries(Ids(true),action);}
 async Task ChangeOptions(){if(changingOptions||busy)return;bool nextAuto=automatic.Checked,nextKeep=approve.Checked;await Run(async delegate{try{if(OptionsChanged!=null)await OptionsChanged(nextAuto,nextKeep);savedAuto=nextAuto;savedKeep=nextKeep;}catch{changingOptions=true;try{automatic.Checked=savedAuto;approve.Checked=savedKeep;}finally{changingOptions=false;}throw;}});}
 async Task Reload(){
  if(LoadEntries==null)return;var loaded=await LoadEntries();if(IsDisposed)return;
  var selected=new HashSet<string>(list.SelectedItems.Cast<ListViewItem>().Select(item=>((DayEntry)item.Tag).Id));
  string top=list.Items.Count>0&&list.TopItem!=null?((DayEntry)list.TopItem.Tag).Id:null;
  entries=loaded;list.BeginUpdate();try{
   list.Items.Clear();foreach(var item in entries){var row=new ListViewItem(new[]{L.Category(item.Category),item.Followup?L.T("Повтор","Follow-up"):L.T("Обычная","Regular"),item.State,item.Text}){Tag=item};list.Items.Add(row);row.Selected=selected.Contains(item.Id);}
  }finally{list.EndUpdate();}
  if(top!=null){var previous=list.Items.Cast<ListViewItem>().FirstOrDefault(item=>((DayEntry)item.Tag).Id==top);if(previous!=null)list.TopItem=previous;}
  FitColumns();feedback.Text=L.T("Фраз: ","Phrases: ")+entries.Count+L.T(". Ctrl+A — выделить все. Прослушивание не расходует фразу.",". Ctrl+A selects all. Preview does not consume a phrase.");
 }
 async Task Run(Func<Task> action,bool reload=true){if(busy||IsDisposed||action==null)return;busy=true;foreach(var control in actions)control.Enabled=false;automatic.Enabled=approve.Enabled=false;try{await action();if(reload&&!IsDisposed)await Reload();}catch(Exception error){if(!IsDisposed)feedback.Text=error.Message;}finally{busy=false;if(!IsDisposed){foreach(var control in actions)control.Enabled=true;automatic.Enabled=approve.Enabled=true;}}}
 protected override void Dispose(bool disposing){if(disposing){refresh.Dispose();selectionMenu.Dispose();}base.Dispose(disposing);}
}
