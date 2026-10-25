using System.Diagnostics;
using System.Text;
using System.Text.Json;
using Avalonia;
using Avalonia.Controls;
using Avalonia.Controls.ApplicationLifetimes;
using Avalonia.Media;
using Avalonia.Media.Imaging;
using Avalonia.Styling;
using Avalonia.Themes.Fluent;
using Avalonia.Threading;

namespace Quote.Avalonia;

internal static class Program
{
    internal static string[] Arguments = [];
    [STAThread] public static void Main(string[] args)
    {
        Arguments = args;
        var builder=AppBuilder.Configure<PrototypeApp>().UsePlatformDetect();
        int at=Array.IndexOf(args,"--renderer");
        if(at>=0&&at+1<args.Length&&args[at+1]=="software")
            builder=builder.With(new Win32PlatformOptions {RenderingMode=[Win32RenderingMode.Software],CompositionMode=[Win32CompositionMode.RedirectionSurface]});
        builder.StartWithClassicDesktopLifetime(args);
    }
}

public sealed class PrototypeApp : Application
{
    public override void Initialize() => Styles.Add(new FluentTheme());
    public override void OnFrameworkInitializationCompleted()
    {
        if (ApplicationLifetime is IClassicDesktopStyleApplicationLifetime life)
        {
            life.ShutdownMode = ShutdownMode.OnExplicitShutdown;
            var host = new Host(this, life, Program.Arguments);
            life.Startup += (_, _) => host.Start();
            life.Exit += (_, _) => host.Dispose();
        }
        base.OnFrameworkInitializationCompleted();
    }
}

internal sealed class Host : IDisposable
{
    readonly Application app;
    readonly IClassicDesktopStyleApplicationLifetime life;
    readonly string common, python;
    readonly bool bench;
    readonly Window pet;
    Window? settings, bubble;
    ListBox? phraseList;
    TextBlock? state;
    TabControl? tabs;
    readonly DispatcherTimer animation;
    readonly TranslateTransform bounce = new();
    readonly List<string> rows = new();
    readonly List<double> tickIntervals = new();
    readonly Stopwatch uptime = Stopwatch.StartNew();
    long lastTick;
    int frame, actions, page;
    bool dark, moving = true, saved;
    string theme = "system";
    Process? worker;
    int progress;
    static readonly object OutputLock = new();
    public Host(Application app, IClassicDesktopStyleApplicationLifetime life, string[] args)
    {
        this.app=app; this.life=life;
        string Arg(string name, string fallback="") {int at=Array.IndexOf(args,name);return at>=0&&at+1<args.Length?args[at+1]:fallback;}
        common=Path.GetFullPath(Arg("--common", Path.Combine(AppContext.BaseDirectory,"common")));
        python=Arg("--python", "python"); bench=args.Contains("--bench");
        pet=new Window {Title="Кавычка · Avalonia · маскот",Width=150,Height=170,CanResize=false,
            WindowDecorations=WindowDecorations.None,ShowInTaskbar=false,ShowActivated=false,Topmost=true,
            TransparencyLevelHint=[WindowTransparencyLevel.Transparent],Background=Brushes.Transparent,
            Position=new PixelPoint(40,80)};
        var image=new Image {Source=new Bitmap(Path.Combine(common,"mascot.png")),Width=150,Height=170,RenderTransform=bounce};
        pet.Content=image;
        pet.PointerPressed+=(_, e)=>{if(e.GetCurrentPoint(pet).Properties.IsLeftButtonPressed)OpenSettings();};
        pet.Opened+=(_,_)=>Native.Overlay(pet,common,true);
        pet.Closing+=(_,e)=>{e.Cancel=true;};
        animation=new DispatcherTimer {Interval=TimeSpan.FromMilliseconds(33)};
        animation.Tick+=(_,_)=>
        {
            long now=Stopwatch.GetTimestamp();
            if(lastTick!=0&&tickIntervals.Count<100000)tickIntervals.Add((now-lastTick)*1000.0/Stopwatch.Frequency);
            lastTick=now;
            if(moving&&pet.IsVisible)bounce.Y=Math.Sin(++frame/7.0)*3;
        };
    }
    public void Start()
    {
        nint before=Native.GetForegroundWindow(); pet.Show(); animation.Start();
        pet.RequestAnimationFrame(_=>
        {
            Emit(new {event_name="ready",stack="avalonia",elapsed_ms=uptime.Elapsed.TotalMilliseconds,
                pid=Environment.ProcessId,foreground_unchanged=before==Native.GetForegroundWindow(),native=Native.Inspect(pet)});
        });
        if(!bench)OpenSettings();
        if(bench) _=Task.Run(ReadCommands);
    }
    internal static void Emit(object value)
    {
        lock(OutputLock){Console.Out.WriteLine(JsonSerializer.Serialize(value));Console.Out.Flush();}
    }
    async Task ReadCommands()
    {
        while(await Console.In.ReadLineAsync() is {} line)
        {
            JsonDocument doc;
            try {doc=JsonDocument.Parse(line);}catch{continue;}
            using(doc)
            {
                var input=doc.RootElement.Clone(); long received=Stopwatch.GetTimestamp();
                await Dispatcher.UIThread.InvokeAsync(()=>
                {
                    int id=input.GetProperty("id").GetInt32();
                    double queueMs=(Stopwatch.GetTimestamp()-received)*1000.0/Stopwatch.Frequency;
                    if(input.GetProperty("command").GetString()=="present")
                    {
                        string target=input.TryGetProperty("target",out var t)?t.GetString()!:"settings";
                        var window=target=="pet"?pet:target=="bubble"?bubble:settings;
                        if(window==null||!window.IsVisible){Emit(new{id,ok=false,error="Target window is hidden"});return;}
                        window.InvalidateVisual();
                        window.RequestAnimationFrame(_=>Dispatcher.UIThread.Post(()=>Emit(new{id,ok=true,queue_ms=queueMs,result=new{frame_signalled=true,target}}),DispatcherPriority.Background));
                        return;
                    }
                    try {var result=Command(input);Emit(new{id,ok=true,queue_ms=queueMs,result});}
                    catch(Exception e){Emit(new{id,ok=false,error=e.ToString()});}
                });
            }
        }
        Dispatcher.UIThread.Post(()=>life.Shutdown());
    }
    object Command(JsonElement input)
    {
        string command=input.GetProperty("command").GetString()!;
        int Number(string key,int fallback=0)=>input.TryGetProperty(key,out var v)?v.GetInt32():fallback;
        string Text(string key,string fallback="")=>input.TryGetProperty(key,out var v)?v.GetString()!:fallback;
        switch(command)
        {
            case "ping": return new {actions,progress,visible=pet.IsVisible};
            case "settings":
                if(Number("show",1)==1)OpenSettings();else{settings?.Close();settings=null;}
                return new {visible=settings?.IsVisible??false};
            case "page":
                OpenSettings();page=Number("page");tabs!.SelectedIndex=page;actions++;return new {page};
            case "theme":
                theme=Text("theme","dark");dark=theme=="dark";app.RequestedThemeVariant=theme=="system"?ThemeVariant.Default:dark?ThemeVariant.Dark:ThemeVariant.Light;
                PaintBackgrounds();
                actions++;return new{theme};
            case "message":
                ShowMessage();return new{visible=bubble!.IsVisible};
            case "confirm": bubble?.Hide();actions++;return new{confirmed=true};
            case "library":
                if(rows.Count==0)for(int i=0;i<10000;i++)rows.Add($"{i+1:00000} · Фраза для перерыва: встань и немного пройдись.");
                OpenSettings();page=2;tabs!.SelectedIndex=2;phraseList!.ItemsSource=rows.Skip(Number("offset")).Take(50).ToArray();
                return new{total=rows.Count,shown=phraseList.ItemCount};
            case "load":
                StartWorker();return new{pid=worker!.Id};
            case "cancel":
                if(worker is {HasExited:false}){worker.StandardInput.WriteLine("stop");worker.StandardInput.Flush();}
                return new{requested=true};
            case "worker":return new{running=worker is {HasExited:false},progress,pid=worker?.Id};
            case "animate":moving=Number("enabled",1)!=0;return new{moving};
            case "native":return Native.Inspect(pet);
            case "visibility":
                if(Number("show",1)==1){pet.Show();Native.Overlay(pet,common,true);}else pet.Hide();return new{visible=pet.IsVisible};
            case "snapshot":
                SaveImage(pet,Text("path")+"-pet.png");if(settings is {IsVisible:true})SaveImage(settings,Text("path")+"-settings.png");
                if(bubble is {IsVisible:true})SaveImage(bubble,Text("path")+"-bubble.png");return new{saved=true};
            case "stats":return new{actions,timer_intervals_ms=tickIntervals.ToArray(),rows=rows.Count,saved};
            case "quit":life.Shutdown();return new{bye=true};
            default:throw new ArgumentException("Unknown command: "+command);
        }
    }
    static void SaveImage(Window window,string path)
    {
        Directory.CreateDirectory(Path.GetDirectoryName(Path.GetFullPath(path))!);
        var size=window.ClientSize;using var bitmap=new RenderTargetBitmap(new PixelSize((int)size.Width,(int)size.Height),new Vector(96,96));
        bitmap.Render(window);bitmap.Save(path,new PngBitmapEncoderOptions());
    }
    Button Action(string text,Action action)
    {
        var b=new Button {Content=text,MinHeight=36};b.Click+=(_,_)=>action();return b;
    }
    StackPanel Column(params Control[] controls)
    {
        var p=new StackPanel {Spacing=14,Margin=new Thickness(24)};foreach(var c in controls)p.Children.Add(c);return p;
    }
    TextBlock Label(string text,int size=14)=>new(){Text=text,FontSize=size,TextWrapping=TextWrapping.Wrap};
    void PaintBackgrounds()
    {
        bool isDark=app.ActualThemeVariant==ThemeVariant.Dark;
        var background=new SolidColorBrush(Color.Parse(isDark?"#191b24":"#f8f9fc"));
        if(settings!=null)settings.Background=background;if(bubble!=null)bubble.Background=background;
    }
    public void OpenSettings()
    {
        if(settings is {IsVisible:true})return;
        settings=new Window {Title="Кавычка · Avalonia · прототип",Width=720,Height=640,MinWidth=640,MinHeight=520,ShowActivated=!bench,Position=new PixelPoint(240,80)};
        var overview=Column(Label("Кавычка",28),Label("Маскот и напоминания. Изолированный прототип оболочки."),
            Label("Ближайшее напоминание · через 5 минут",18),
            Action("Показать сообщение",ShowMessage),Action("Запустить фоновую нагрузку",StartWorker),
            Action("Остановить нагрузку",()=>{if(worker is {HasExited:false}){worker.StandardInput.WriteLine("stop");worker.StandardInput.Flush();}}),
            state=Label("Готово · модели не используются"),Action("Закрыть прототип",()=>life.Shutdown()));
        var themeChoice=new ComboBox {ItemsSource=new[]{"Как в системе","Светлая","Тёмная"},SelectedIndex=theme=="system"?0:dark?2:1};
        themeChoice.SelectionChanged+=(_,_)=>{theme=themeChoice.SelectedIndex==0?"system":themeChoice.SelectedIndex==1?"light":"dark";dark=theme=="dark";app.RequestedThemeVariant=theme=="system"?ThemeVariant.Default:dark?ThemeVariant.Dark:ThemeVariant.Light;};
        var schedule=Column(Label("Ритм дня",26),Label("Тема"),themeChoice,Label("Напоминать каждые, минут"),new NumericUpDown {Minimum=5,Maximum=120,Value=60},new CheckBox {Content="Озвучивать сообщения",IsChecked=true},new CheckBox {Content="Сигнал перед озвучкой",IsChecked=true},new TextBox {PlaceholderText="Как тебя называть"},Action("Сохранить",()=>{saved=true;}));
        phraseList=new ListBox {Height=430};
        var library=Column(Label("Фразы на сегодня",26),Label("История: 10 000 строк · на странице 50"),phraseList);
        var models=Column(Label("Текст и голос",26),Label("Текстовая модель"),new ComboBox {ItemsSource=new[]{"Готовые фразы","Gemma","Qwen"},SelectedIndex=0},Label("Голосовая модель"),new ComboBox {ItemsSource=new[]{"Голос Windows","CosyVoice","Qwen TTS"},SelectedIndex=0},Label("Загрузки отключены: это прототип интерфейса."));
        tabs=new TabControl {ItemsSource=new[]{new TabItem {Header="Обзор",Content=overview},new TabItem {Header="Расписание",Content=schedule},new TabItem {Header="Библиотека",Content=library},new TabItem {Header="Модели",Content=models}},SelectedIndex=page};
        settings.Content=tabs;settings.Closed+=(_,_)=>{settings=null;tabs=null;phraseList=null;state=null;};PaintBackgrounds();settings.Show();
    }
    public void ShowMessage()
    {
        if(bubble==null)
        {
            bubble=new Window {Title="Кавычка · Avalonia · сообщение",Width=400,Height=200,CanResize=false,WindowDecorations=WindowDecorations.None,ShowInTaskbar=false,ShowActivated=false,Topmost=true,Position=new PixelPoint(40,280)};
            bubble.Content=Column(Label("Кавычка",18),Label("Стул сегодня уже поработал. Встань и дай ему короткий отпуск."),
                new StackPanel {Orientation=global::Avalonia.Layout.Orientation.Horizontal,Spacing=12,Children={Action("Я встала / встал",()=>bubble!.Hide()),Action("Стоп голос",()=>{})}});
            bubble.Opened+=(_,_)=>Native.Overlay(bubble,common,false);
        }
        PaintBackgrounds();bubble.Show();Native.Overlay(bubble,common,false);
    }
    void StartWorker()
    {
        if(worker is {HasExited:false})return;
        worker?.Dispose();
        worker=new Process {StartInfo=new ProcessStartInfo(python){UseShellExecute=false,CreateNoWindow=true,RedirectStandardInput=true,RedirectStandardOutput=true,RedirectStandardError=true}};
        worker.StartInfo.ArgumentList.Add("-B");worker.StartInfo.ArgumentList.Add(Path.Combine(common,"load_worker.py"));
        worker.Start();var running=worker;
        _=Task.Run(async()=>{while(await running.StandardOutput.ReadLineAsync() is {} line)try{using var d=JsonDocument.Parse(line);if(d.RootElement.TryGetProperty("value",out var v)){int n=v.GetInt32();Dispatcher.UIThread.Post(()=>{progress=n;if(state!=null)state.Text=$"Фоновая задача · {n}%";});}}catch(JsonException){}});
        _=Task.Run(async()=>await running.StandardError.ReadToEndAsync());
    }
    public void Dispose()
    {
        animation.Stop();
        if(worker is {HasExited:false})worker.Kill(true);
        worker?.Dispose();
    }
}
