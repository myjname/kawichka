using System;
using System.Collections.Generic;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Windows.Forms;

public static partial class StudioColors {
 public static GraphicsPath Round(RectangleF bounds,float radius){var p=new GraphicsPath();float d=radius*2;p.AddArc(bounds.X,bounds.Y,d,d,180,90);p.AddArc(bounds.Right-d,bounds.Y,d,d,270,90);p.AddArc(bounds.Right-d,bounds.Bottom-d,d,d,0,90);p.AddArc(bounds.X,bounds.Bottom-d,d,d,90,90);p.CloseFigure();return p;}
}

public class StudioButton:Button {
 bool hover;
 public bool Primary=true,Selected;
 public StudioButton(){FlatStyle=FlatStyle.Flat;FlatAppearance.BorderSize=0;Cursor=Cursors.Hand;DoubleBuffered=true;Height=42;Font=new Font("Segoe UI",10,FontStyle.Regular);BackColor=StudioColors.Background;MouseEnter+=delegate{hover=true;Invalidate();};MouseLeave+=delegate{hover=false;Invalidate();};}
 protected override void OnPaint(PaintEventArgs e){var g=e.Graphics;g.SmoothingMode=SmoothingMode.AntiAlias;g.Clear(BackColor);Color fill=!Enabled?StudioColors.Soft:Primary?(hover?StudioColors.AccentHover:StudioColors.Accent):Selected?StudioColors.Soft:hover?StudioColors.Hover:BackColor;
  using(var path=StudioColors.Round(new RectangleF(0,0,Width-1,Height-1),10))using(var brush=new SolidBrush(fill))g.FillPath(brush,path);
  TextRenderer.DrawText(g,Text,Font,new Rectangle(Primary?12:16,0,Width-(Primary?24:28),Height),Enabled?(Primary?StudioColors.OnAccent:Selected?StudioColors.Accent:StudioColors.Ink):StudioColors.Muted,(Primary?TextFormatFlags.HorizontalCenter:TextFormatFlags.Left)|TextFormatFlags.VerticalCenter|TextFormatFlags.EndEllipsis);
  if(Focused)ControlPaint.DrawFocusRectangle(g,new Rectangle(5,5,Width-10,Height-10),StudioColors.Accent,fill);
 }
}

public class StudioToggle:CheckBox {
 public StudioToggle(){SetStyle(ControlStyles.UserPaint|ControlStyles.OptimizedDoubleBuffer|ControlStyles.AllPaintingInWmPaint,true);Cursor=Cursors.Hand;Height=48;Width=550;}
 protected override void OnPaint(PaintEventArgs e){var g=e.Graphics;g.SmoothingMode=SmoothingMode.AntiAlias;g.Clear(BackColor);using(var path=StudioColors.Round(new RectangleF(0,12,40,22),11))using(var brush=new SolidBrush(Checked?StudioColors.Accent:StudioColors.Track))g.FillPath(brush,path);using(var brush=new SolidBrush(Checked?StudioColors.OnAccent:Color.White))g.FillEllipse(brush,Checked?21:3,15,16,16);TextRenderer.DrawText(g,Text,Font,new Rectangle(55,0,Width-55,Height),StudioColors.Ink,TextFormatFlags.Left|TextFormatFlags.VerticalCenter|TextFormatFlags.WordBreak);if(Focused)ControlPaint.DrawFocusRectangle(g,new Rectangle(51,3,Width-55,Height-6));}
}

public sealed class StudioCombo:ComboBox {
 public StudioCombo(){FlatStyle=FlatStyle.Flat;MouseEnter+=delegate{Invalidate();};MouseLeave+=delegate{Invalidate();};}
 void Frame(Graphics g){
  int arrow=Math.Max(24,Height-4);Color fill=Enabled?StudioColors.Surface:StudioColors.Background;
  using(var brush=new SolidBrush(fill))g.FillRectangle(brush,Width-arrow,1,arrow-1,Height-2);
  using(var border=new Pen(Focused?StudioColors.Accent:StudioColors.Border)){g.DrawRectangle(border,0,0,Width-1,Height-1);g.DrawRectangle(border,1,1,Width-3,Height-3);}
  float x=Width-arrow/2f,y=Height/2f;using(var pen=new Pen(Enabled?StudioColors.Muted:StudioColors.Track,1.5f)){g.DrawLines(pen,new[]{new PointF(x-4,y-2),new PointF(x,y+2),new PointF(x+4,y-2)});}
 }
 protected override void WndProc(ref Message m){base.WndProc(ref m);if(m.Msg==0xF||m.Msg==0x85){using(var g=Graphics.FromHwnd(Handle))Frame(g);}else if((m.Msg==0x317||m.Msg==0x318)&&m.WParam!=IntPtr.Zero){using(var g=Graphics.FromHdc(m.WParam))Frame(g);}}
}

public sealed class StudioCard:Panel {
 public readonly Label Heading,Detail;
 public StudioCard(string heading,string detail){Size=new Size(525,115);Margin=new Padding(0,6,0,14);BackColor=StudioColors.Background;DoubleBuffered=true;
  Heading=new Label{Text=heading,Location=new Point(20,16),Size=new Size(485,25),Font=new Font("Segoe UI",10,FontStyle.Bold),ForeColor=StudioColors.Accent,BackColor=StudioColors.Surface};
  Detail=new Label{Text=detail,Location=new Point(20,47),Size=new Size(485,54),Font=new Font("Segoe UI",11),ForeColor=StudioColors.Ink,BackColor=StudioColors.Surface};Controls.Add(Heading);Controls.Add(Detail);
 }
 protected override void OnPaint(PaintEventArgs e){base.OnPaint(e);e.Graphics.SmoothingMode=SmoothingMode.AntiAlias;using(var p=StudioColors.Round(new RectangleF(0,0,Width-1,Height-1),14))using(var b=new SolidBrush(StudioColors.Surface))using(var line=new Pen(StudioColors.Border)){e.Graphics.FillPath(b,p);e.Graphics.DrawPath(line,p);}}
}

public sealed class StudioDays:FlowLayoutPanel {
 readonly Dictionary<int,StudioButton> choices=new Dictionary<int,StudioButton>();
 public StudioDays(int[] days){Size=new Size(535,50);WrapContents=false;Margin=new Padding(0,0,0,14);string[] names=L.Language=="ru"?new[]{"Пн","Вт","Ср","Чт","Пт","Сб","Вс"}:new[]{"Mon","Tue","Wed","Thu","Fri","Sat","Sun"};for(int i=0;i<7;i++){int day=(i+1)%7;var b=new StudioButton{Text=names[i],Width=64,Height=42,Primary=false,Selected=Array.IndexOf(days,day)>=0,Margin=new Padding(0,0,10,0)};choices[day]=b;Controls.Add(b);b.Click+=delegate{b.Selected=!b.Selected;b.Invalidate();};}}
 public int[] Days {get{var result=new List<int>();foreach(var pair in choices)if(pair.Value.Selected)result.Add(pair.Key);return result.ToArray();}}
}

public sealed class StudioClock:UserControl {
 readonly MaskedTextBox input;
 public StudioClock(int minutes){Height=36;BackColor=StudioColors.Surface;ForeColor=StudioColors.Ink;DoubleBuffered=true;AccessibleName=L.T("Время, часы и минуты","Time, hours and minutes");
  input=new MaskedTextBox("00:00"){BorderStyle=BorderStyle.None,Location=new Point(10,7),Width=90,BackColor=BackColor,ForeColor=ForeColor,Font=new Font("Segoe UI",12),TextMaskFormat=MaskFormat.IncludeLiterals};Controls.Add(input);Minutes=minutes;
  input.KeyDown+=delegate(object sender,KeyEventArgs e){if(e.KeyCode==Keys.Up||e.KeyCode==Keys.Down){Step((e.KeyCode==Keys.Up?1:-1)*(e.Control?60:1));e.Handled=true;e.SuppressKeyPress=true;}};
 }
 public int Minutes{get{TimeSpan time;if(!TimeSpan.TryParseExact(input.Text,"hh\\:mm",System.Globalization.CultureInfo.InvariantCulture,out time)||time.TotalMinutes<0||time.TotalMinutes>=1440)throw new ArgumentException(L.T("Укажи время от 00:00 до 23:59","Enter a time between 00:00 and 23:59"));return (int)time.TotalMinutes;}set{value=(value%1440+1440)%1440;input.Text=(value/60).ToString("00")+":"+(value%60).ToString("00");}}
 void Step(int amount){try{Minutes+=amount;}catch(ArgumentException){Minutes=0;}}
 protected override void OnMouseDown(MouseEventArgs e){base.OnMouseDown(e);input.Focus();if(e.X>=Width-30)Step(e.Y<Height/2?1:-1);}
 protected override void OnPaint(PaintEventArgs e){base.OnPaint(e);using(var p=new Pen(StudioColors.Border))e.Graphics.DrawRectangle(p,0,0,Width-1,Height-1);using(var brush=new SolidBrush(StudioColors.Muted)){float x=Width-16;e.Graphics.FillPolygon(brush,new[]{new PointF(x-4,13),new PointF(x,9),new PointF(x+4,13)});e.Graphics.FillPolygon(brush,new[]{new PointF(x-4,23),new PointF(x,27),new PointF(x+4,23)});}}
}

public sealed class StudioPages:Panel {
 readonly FlowLayoutPanel nav;
 readonly Panel host,footer;
 readonly List<FlowLayoutPanel> pages=new List<FlowLayoutPanel>();
 readonly List<NightScroll> scrolls=new List<NightScroll>();
 readonly List<Panel> pageFrames=new List<Panel>();
 readonly List<StudioButton> buttons=new List<StudioButton>();
 bool initialized;
 public StudioPages(){BackColor=StudioColors.Background;host=new Panel{Dock=DockStyle.Fill,BackColor=StudioColors.Background};Controls.Add(host);var sidebar=new Panel{Dock=DockStyle.Left,Width=195,BackColor=StudioColors.Sidebar};Controls.Add(sidebar);
  var name=new Label{Text="❝",Location=new Point(23,20),Size=new Size(45,50),Font=new Font("Georgia",33,FontStyle.Bold),ForeColor=StudioColors.Accent};sidebar.Controls.Add(name);sidebar.Controls.Add(new Label{Text=L.T("Кавычка","Kavychka"),Location=new Point(72,33),Size=new Size(116,30),Font=new Font("Segoe UI",15,FontStyle.Bold),ForeColor=StudioColors.Ink});
  nav=new FlowLayoutPanel{Location=new Point(14,100),Size=new Size(168,330),FlowDirection=FlowDirection.TopDown,WrapContents=false,BackColor=sidebar.BackColor};sidebar.Controls.Add(nav);
  footer=new Panel{Dock=DockStyle.Bottom,Height=133,BackColor=sidebar.BackColor};sidebar.Controls.Add(footer);
  footer.Controls.Add(new Label{Text="v2.0  ·  "+L.T("на твоём компьютере","on your computer"),Location=new Point(22,105),Size=new Size(165,25),Font=new Font("Segoe UI",8),ForeColor=StudioColors.Muted});
 }
 public FlowLayoutPanel AddPage(string name){var p=new FlowLayoutPanel{Width=600,MinimumSize=new Size(600,0),MaximumSize=new Size(600,0),AutoSize=true,AutoSizeMode=AutoSizeMode.GrowAndShrink,FlowDirection=FlowDirection.TopDown,WrapContents=false,Padding=new Padding(32,24,20,24),BackColor=StudioColors.Background,ForeColor=StudioColors.Ink};pages.Add(p);var scroll=new NightScroll{BackColor=StudioColors.Background,ForeColor=StudioColors.Ink};scroll.Controls.Add(p);scrolls.Add(scroll);var frame=new Panel{Dock=DockStyle.Fill,BackColor=StudioColors.Background};pageFrames.Add(frame);frame.Controls.Add(scroll);host.Controls.Add(frame);int index=pages.Count-1;var b=new StudioButton{Text=name,Primary=false,Width=166,Height=45,BackColor=StudioColors.Sidebar,Margin=new Padding(0,0,0,8)};buttons.Add(b);nav.Controls.Add(b);b.Click+=delegate{Select(index);};Select(0);return p;}
 public void InitializeScrolling(){if(initialized)return;initialized=true;for(int i=0;i<pages.Count;i++)scrolls[i].SetContent(pages[i]);}
 public void Select(int index){for(int i=0;i<pages.Count;i++){pageFrames[i].Visible=scrolls[i].Visible=i==index;buttons[i].Selected=i==index;buttons[i].Invalidate();}if(index<pages.Count)pageFrames[index].BringToFront();}
 public void PinFooter(FlowLayoutPanel page,Control controls){int index=pages.IndexOf(page);if(index<0)throw new ArgumentException(AppText.T("Неизвестная страница настроек","Unknown settings page"));controls.Dock=DockStyle.Bottom;pageFrames[index].Controls.Add(controls);scrolls[index].BringToFront();}
 public StudioButton AddFooter(string text,int index,Action action){var b=new StudioButton{Text=text,Primary=false,Width=168,Height=39,BackColor=StudioColors.Sidebar,Location=new Point(14,8+index*42),Font=new Font("Segoe UI",9)};footer.Controls.Add(b);b.Click+=delegate{action();};return b;}
}
