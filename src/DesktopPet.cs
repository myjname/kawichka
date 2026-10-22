using System;
using System.Drawing;
using System.Drawing.Drawing2D;
using System.Drawing.Imaging;
using System.IO;
using System.Runtime.InteropServices;
using System.Windows.Forms;

public class FloatingWindow:Form {
 protected override bool ShowWithoutActivation {get{return true;}}
 protected override CreateParams CreateParams {get{var p=base.CreateParams;p.ExStyle|=0x08000000|0x00000080;return p;}}
 protected override void WndProc(ref Message m){if(m.Msg==0x21){m.Result=new IntPtr(3);return;}base.WndProc(ref m);}
 public FloatingWindow(){FormBorderStyle=FormBorderStyle.None;ShowInTaskbar=false;TopMost=true;DoubleBuffered=true;StartPosition=FormStartPosition.Manual;}
}

public sealed class PetForm:FloatingWindow {
 readonly System.Windows.Forms.Timer animation;
 Image visual;
 MemoryStream visualStream;
 AlphaSurface surface;
 string visualLayout="auto";
 Point dragStart,windowStart;
 bool dragging,moved;
 int frame;
 public bool Paused,Speaking,Teasing,Happy;
 public string PauseText="";
 public Action Clicked,PositionChanged;
 public Action<Exception> RenderingFailed;
 public string RenderingError {get;private set;}
 protected override CreateParams CreateParams {get{var p=base.CreateParams;p.ExStyle|=0x00080000;return p;}}
 public PetForm() {
  ClientSize=new Size(150,170);BackColor=Color.Black;
  animation=new System.Windows.Forms.Timer{Interval=110};animation.Tick+=delegate{frame++;PresentFrame();};animation.Start();
  MouseDown+=delegate(object sender,MouseEventArgs e){if(e.Button!=MouseButtons.Left)return;dragging=true;moved=false;dragStart=Cursor.Position;windowStart=Location;Capture=true;};
  MouseMove+=delegate {if(!dragging)return;Point delta=new Point(Cursor.Position.X-dragStart.X,Cursor.Position.Y-dragStart.Y);if(Math.Abs(delta.X)+Math.Abs(delta.Y)>5)moved=true;Location=new Point(windowStart.X+delta.X,windowStart.Y+delta.Y);};
  MouseUp+=delegate(object sender,MouseEventArgs e){if(!dragging)return;dragging=false;Capture=false;if(moved){ClampToScreen();if(PositionChanged!=null)PositionChanged();}else if(Clicked!=null)Clicked();};
 }
 public void ClampToScreen(){var area=Screen.FromPoint(new Point(Left+Width/2,Top+Height/2)).WorkingArea;Left=Math.Max(area.Left,Math.Min(area.Right-Width,Left));Top=Math.Max(area.Top,Math.Min(area.Bottom-Height,Top));}
 public void SetVisual(string path,string layout="auto"){
  Image next=null;MemoryStream stream=null;
  try{if(!string.IsNullOrEmpty(path)){stream=new MemoryStream(File.ReadAllBytes(path));next=Image.FromStream(stream);if(layout=="codex"&&!SpriteAtlas.Matches(next))throw new ArgumentException(AppText.T("Лист спрайтов Codex должен иметь размер 1536 × 1872","Codex sprite sheet must be 1536 x 1872"));}
   ClearVisual();visual=next;visualStream=stream;next=null;stream=null;visualLayout=layout;
   if(visual!=null&&ImageAnimator.CanAnimate(visual))ImageAnimator.Animate(visual,AnimateFrame);
  }finally{if(next!=null)next.Dispose();if(stream!=null)stream.Dispose();}
  PresentFrame();Invalidate();
 }
 void ClearVisual(){if(visual!=null){ImageAnimator.StopAnimate(visual,AnimateFrame);visual.Dispose();visual=null;}if(visualStream!=null){visualStream.Dispose();visualStream=null;}}
 void AnimateFrame(object sender,EventArgs e){if(!IsDisposed&&IsHandleCreated)try{BeginInvoke((Action)delegate{if(!IsDisposed)PresentFrame();});}catch(InvalidOperationException){}}
 protected override void OnVisibleChanged(EventArgs e){base.OnVisibleChanged(e);if(Visible)PresentFrame();}
 protected override void OnSizeChanged(EventArgs e){base.OnSizeChanged(e);if(surface!=null){surface.Dispose();surface=null;}PresentFrame();}
 protected override void OnHandleDestroyed(EventArgs e){if(surface!=null){surface.Dispose();surface=null;}base.OnHandleDestroyed(e);}
 void PresentFrame(){
  if(IsDisposed||!Visible||!IsHandleCreated||Width<1||Height<1)return;
  try{if(surface==null)surface=new AlphaSurface(Width,Height);using(var g=Graphics.FromImage(surface.Frame)){g.Clear(Color.Transparent);DrawPet(g);}surface.Present(Handle,Location);RenderingError="";}
  catch(Exception error){if(RenderingError!=error.Message){RenderingError=error.Message;if(RenderingFailed!=null)RenderingFailed(error);}}
 }
 public Bitmap RenderSnapshot(){var bitmap=new Bitmap(Width,Height,PixelFormat.Format32bppArgb);using(var g=Graphics.FromImage(bitmap)){g.Clear(Color.Transparent);DrawPet(g);}return bitmap;}
 protected override void OnPaintBackground(PaintEventArgs e){e.Graphics.Clear(Color.Transparent);}
 protected override void OnPaint(PaintEventArgs e){base.OnPaint(e);DrawPet(e.Graphics);}
 void DrawPet(Graphics g){
  var state=g.Save();try{g.SmoothingMode=SmoothingMode.AntiAlias;g.TextRenderingHint=System.Drawing.Text.TextRenderingHint.AntiAliasGridFit;
  float scale=Width/150f;g.ScaleTransform(scale,scale);float bounce=Paused?0:(float)Math.Sin(frame/(Happy?3.0:7.0))*(Happy?4:3);
  if(visual!=null){ImageAnimator.UpdateFrames(visual);Rectangle source=new Rectangle(0,0,visual.Width,visual.Height);
   if(visualLayout!="image"&&SpriteAtlas.Matches(visual))source=SpriteAtlas.Frame(frame*110L,Paused,Speaking,Teasing,Happy);
   float fit=Math.Min(132f/source.Width,133f/source.Height);var target=new RectangleF((150-source.Width*fit)/2,10+bounce,source.Width*fit,source.Height*fit);g.DrawImage(visual,target,source,GraphicsUnit.Pixel);
  } else {
   using(var shadow=new SolidBrush(Color.FromArgb(38,40,36,46)))g.FillEllipse(shadow,22,135,106,12);
   using(var fill=new SolidBrush(Paused?Color.FromArgb(155,159,164):Teasing?Color.FromArgb(243,174,119):Color.FromArgb(250,208,119))) {
    g.FillEllipse(fill,17,30+bounce,51,72);g.FillEllipse(fill,79,30+bounce,51,72);
    PointF[] a={new PointF(30,87+bounce),new PointF(59,83+bounce),new PointF(39,126+bounce),new PointF(22,128+bounce)};
    PointF[] b={new PointF(93,87+bounce),new PointF(122,83+bounce),new PointF(102,126+bounce),new PointF(85,128+bounce)};g.FillPolygon(fill,a);g.FillPolygon(fill,b);
   }
   using(var eye=new Pen(Color.FromArgb(51,42,49),4)){
    if(Happy&&!Paused){g.DrawArc(eye,new RectangleF(35,57+bounce,13,10),180,180);g.DrawArc(eye,new RectangleF(96,57+bounce,13,10),180,180);}
    else if(frame%45<3||Paused){g.DrawLine(eye,35,61+bounce,48,61+bounce);g.DrawLine(eye,96,61+bounce,109,61+bounce);}else{g.DrawEllipse(eye,40,55+bounce,3,9);g.DrawEllipse(eye,101,55+bounce,3,9);}
    if(Speaking&&!Paused){using(var mouth=new SolidBrush(eye.Color))g.FillEllipse(mouth,69,81+bounce,12,4+frame%3*3);}
    else g.DrawArc(eye,new RectangleF(63,80+bounce,23,Happy&&!Paused?13:8),0,180);
   }
   using(var cheek=new SolidBrush(Color.FromArgb(231,143,139))){g.FillEllipse(cheek,27,76+bounce,12,6);g.FillEllipse(cheek,111,76+bounce,12,6);}
  }
  if(Paused){using(var bar=new Pen(Color.FromArgb(145,151,166),5)){for(int x=12;x<145;x+=25)g.DrawLine(bar,x,17,x,140);g.DrawLine(bar,9,25,140,25);g.DrawLine(bar,9,131,140,131);}
   using(var bg=new SolidBrush(Color.FromArgb(40,37,45)))g.FillRectangle(bg,22,145,108,22);
   using(var font=new Font("Segoe UI",9,FontStyle.Bold))using(var brush=new SolidBrush(Color.White))using(var format=new StringFormat{Alignment=StringAlignment.Center})g.DrawString(PauseText,font,brush,new RectangleF(23,146,108,22),format);
  }
  }finally{g.Restore(state);}
 }
 protected override void Dispose(bool disposing){if(disposing){animation.Dispose();ClearVisual();if(surface!=null){surface.Dispose();surface=null;}}base.Dispose(disposing);}
}

public sealed class PetMessage:FloatingWindow {
 readonly Label message;
 readonly StudioButton accept,mute;
 public Action Confirmed,StopVoice;
 public PetMessage(string text,string confirm,string stop) {
  BackColor=StudioColors.Background;ForeColor=StudioColors.Ink;ClientSize=new Size(400,205);
  Controls.Add(new Label{Text=L.T("Кавычка","Kavychka"),Location=new Point(20,16),Size=new Size(360,22),Font=new Font("Segoe UI",9,FontStyle.Bold),ForeColor=StudioColors.Accent});
  message=new Label{Text=text,Location=new Point(20,48),Size=new Size(360,85),Font=new Font("Segoe UI",11),ForeColor=ForeColor};Controls.Add(message);
  accept=new StudioButton{Text=confirm,Location=new Point(18,149),Size=new Size(220,40)};Controls.Add(accept);accept.Click+=delegate{Hide();if(Confirmed!=null)Confirmed();};
  mute=new StudioButton{Text=stop,Primary=false,Location=new Point(246,149),Size=new Size(135,40)};Controls.Add(mute);mute.Click+=delegate{if(StopVoice!=null)StopVoice();};SetMessage(text);
 }
 public void SetMessage(string text){message.Text=text;int height=TextRenderer.MeasureText(text,message.Font,new Size(message.Width,int.MaxValue),TextFormatFlags.WordBreak).Height+12;message.Height=Math.Max(65,Math.Min(320,height));accept.Top=mute.Top=message.Bottom+16;ClientSize=new Size(400,accept.Bottom+18);using(var shape=StudioColors.Round(new RectangleF(0,0,Width,Height),18)){var old=Region;Region=new Region(shape);if(old!=null)old.Dispose();}}
 protected override void OnPaint(PaintEventArgs e){base.OnPaint(e);e.Graphics.SmoothingMode=SmoothingMode.AntiAlias;using(var shape=StudioColors.Round(new RectangleF(0,0,Width-1,Height-1),18))using(var line=new Pen(StudioColors.Border))e.Graphics.DrawPath(line,shape);}
 public void AnchorTo(PetForm pet){var area=Screen.FromControl(pet).WorkingArea;int x=pet.Left-Width+pet.Width,y=pet.Top-Height-6;if(y<area.Top)y=pet.Bottom+6;Location=new Point(Math.Max(area.Left,Math.Min(area.Right-Width,x)),Math.Max(area.Top,Math.Min(area.Bottom-Height,y)));}
}
