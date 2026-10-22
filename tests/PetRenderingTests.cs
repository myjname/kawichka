using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.Drawing.Imaging;
using System.IO;
using System.Reflection;
using System.Runtime.InteropServices;
using System.Threading.Tasks;
using System.Windows.Forms;

public static class PetRenderingTests {
 static readonly BindingFlags Private=BindingFlags.Instance|BindingFlags.NonPublic;
 static int passed;
 [DllImport("user32.dll")]static extern uint GetGuiResources(IntPtr process,uint kind);
 [DllImport("user32.dll")]static extern int GetWindowLong(IntPtr window,int index);
 static void Check(bool yes,string name){if(!yes)throw new Exception(name);Console.WriteLine("PASS "+name);passed++;}
 static T Get<T>(object owner,string name){return (T)owner.GetType().GetField(name,Private).GetValue(owner);}
 static void Present(PetForm pet){typeof(PetForm).GetMethod("PresentFrame",Private).Invoke(pet,null);}
 static void Composite(Bitmap frame,Color background,string file){using(var image=new Bitmap(frame.Width+40,frame.Height+40)){using(var g=Graphics.FromImage(image)){g.Clear(background);g.DrawImageUnscaled(frame,20,20);}image.Save(file);}}
 static async Task Run(string root,string gif){
  Directory.CreateDirectory(root);L.Language="en";
  using(var pet=new PetForm()){
   pet.Location=new Point(40,40);pet.Show();Present(pet);
   Check(string.IsNullOrEmpty(pet.RenderingError)&&Get<AlphaSurface>(pet,"surface")!=null,"real layered window accepts the premultiplied frame");
   int style=GetWindowLong(pet.Handle,-20);
   Check((style&0x08080080)==0x08080080&&pet.TransparencyKey.IsEmpty,"native mascot keeps layered, no-activation and tool-window styles without a color key");
   using(var image=pet.RenderSnapshot()){
    int transparent=0,partial=0,magenta=0;
    for(int y=0;y<image.Height;y++)for(int x=0;x<image.Width;x++){Color c=image.GetPixel(x,y);if(c.A==0)transparent++;else if(c.A<255)partial++;if(c.A>0&&c.R>200&&c.B>170&&c.G<100)magenta++;}
    Check(transparent>image.Width*image.Height/2&&partial>100&&magenta==0,"default mascot has transparent background and antialiased edges without pink pixels");
    image.Save(Path.Combine(root,"mascot-alpha.png"));Composite(image,Color.FromArgb(248,249,252),Path.Combine(root,"mascot-light.png"));Composite(image,Color.FromArgb(25,27,36),Path.Combine(root,"mascot-dark.png"));
   }
   var states=new HashSet<string>();foreach(string state in new[]{"idle","speaking","teasing","happy"}){pet.Speaking=state=="speaking";pet.Teasing=state=="teasing";pet.Happy=state=="happy";using(var image=pet.RenderSnapshot()){using(var stream=new MemoryStream()){image.Save(stream,ImageFormat.Png);states.Add(Convert.ToBase64String(stream.ToArray()));}image.Save(Path.Combine(root,"mascot-"+state+".png"));}}
   Check(states.Count==4,"default idle, speaking, teasing and happy expressions render differently");pet.Speaking=pet.Teasing=pet.Happy=false;
   string png=Path.Combine(root,"half-alpha.png");using(var image=new Bitmap(20,20,PixelFormat.Format32bppArgb)){using(var g=Graphics.FromImage(image)){g.Clear(Color.FromArgb(128,200,100,50));}image.Save(png);}
   pet.SetVisual(png);File.Delete(png);Present(pet);
   using(var image=pet.RenderSnapshot()){Color c=image.GetPixel(75,75);Check(Math.Abs(c.A-128)<=1&&Math.Abs(c.R-200)<=2,"imported PNG keeps partial alpha after the original file is removed");}
   var surface=Get<AlphaSurface>(pet,"surface");IntPtr bits=Get<IntPtr>(surface,"bits");int pixel=(75*surface.Frame.Width+75)*4;var bytes=new byte[4];Marshal.Copy(IntPtr.Add(bits,pixel),bytes,0,4);
   Check(Math.Abs(bytes[0]-25)<=1&&Math.Abs(bytes[1]-50)<=1&&Math.Abs(bytes[2]-100)<=1&&bytes[3]==128,"native DIB stores premultiplied BGRA rather than straight alpha");
   string invalid=Path.Combine(root,"broken.png");File.WriteAllText(invalid,"invalid image");bool refused=false;try{pet.SetVisual(invalid);}catch(ArgumentException){refused=true;}
   using(var image=pet.RenderSnapshot())Check(refused&&Math.Abs(image.GetPixel(75,75).A-128)<=1,"a broken replacement leaves the previous visual usable");
   string animation=Path.Combine(root,"animated.gif");File.Copy(gif,animation);pet.SetVisual(animation);File.Delete(animation);
   var colors=new HashSet<int>();for(int i=0;i<12;i++){using(var image=pet.RenderSnapshot()){var c=image.GetPixel(75,75);colors.Add(c.R>c.B?1:2);}await Task.Delay(100);}
   Check(colors.Count==2&&string.IsNullOrEmpty(pet.RenderingError),"GIF continues to animate from owned memory after the source file is removed");
   pet.SetVisual("");pet.Paused=true;pet.PauseText="15 min";Present(pet);using(var image=pet.RenderSnapshot())image.Save(Path.Combine(root,"mascot-paused.png"));
   Check(string.IsNullOrEmpty(pet.RenderingError),"pause bars and countdown render through the alpha surface");
   using(var process=Process.GetCurrentProcess()){
    for(int i=0;i<10;i++)Present(pet);uint before=GetGuiResources(process.Handle,0);var previous=Get<AlphaSurface>(pet,"surface");
    for(int i=0;i<200;i++)Present(pet);
    Check(object.ReferenceEquals(previous,Get<AlphaSurface>(pet,"surface"))&&GetGuiResources(process.Handle,0)<=before+2,"animation reuses native buffers without growing GDI resources");
    for(int i=0;i<30;i++){pet.ClientSize=new Size(150+i%2,170+i%2);Present(pet);}
    Check(string.IsNullOrEmpty(pet.RenderingError)&&GetGuiResources(process.Handle,0)<=before+2,"resizing replaces and releases the previous native surface");
   }
   pet.Hide();Check(!pet.Visible,"mascot can hide without closing the notification host");pet.Show();Present(pet);Check(string.IsNullOrEmpty(pet.RenderingError),"showing the mascot again restores its alpha surface");
  }
  using(var process=Process.GetCurrentProcess()){uint before=GetGuiResources(process.Handle,0);for(int i=0;i<100;i++)using(var surface=new AlphaSurface(20,20)){using(var g=Graphics.FromImage(surface.Frame))g.Clear(Color.Transparent);}Check(GetGuiResources(process.Handle,0)<=before+1,"disposing surfaces releases their bitmap and DC handles");}
 }
 [STAThread]public static int Main(string[] args){if(args.Length!=2)return 2;Application.EnableVisualStyles();Application.SetCompatibleTextRenderingDefault(false);int code=1;using(var host=new Form{Opacity=0,ShowInTaskbar=false,Width=1,Height=1}){host.Shown+=async delegate{try{await Run(args[0],args[1]);Console.WriteLine("PASS "+passed+" mascot rendering checks");code=0;}catch(Exception error){Console.Error.WriteLine(error);}finally{host.Close();}};Application.Run(host);}return code;}
}
