#![windows_subsystem = "windows"]

mod native;
slint::include_modules!();

use std::{cell::RefCell,rc::Rc,path::{Path,PathBuf},io::{BufRead,Write},process::{Child,Command,Stdio},sync::{Arc,Mutex},time::{Instant,Duration}};
use serde_json::{Value,json};
use slint::{ComponentHandle,ModelRc,VecModel};
use slint::winit_030::winit::platform::windows::WindowAttributesExtWindows;
use std::os::windows::process::CommandExt;

static OUTPUT:Mutex<()>=Mutex::new(());
thread_local! {static FRAME_WAITERS:RefCell<Vec<(Value,Instant,String)>>=const {RefCell::new(Vec::new())};}
fn frame_complete(kind:&str) {
    FRAME_WAITERS.with(|slot|{
        let mut waiters=slot.borrow_mut();let mut remaining=vec![];
        for (id,started,target) in waiters.drain(..){
            if target==kind{emit(json!({"id":id,"ok":true,"queue_ms":0,"result":{"frame_signalled":true,"target":target,"wait_ms":started.elapsed().as_secs_f64()*1000.}}));}
            else{remaining.push((id,started,target));}
        }
        *waiters=remaining;
    });
}
fn emit(value:Value) {
    let _guard=OUTPUT.lock().unwrap();let mut out=std::io::stdout().lock();let _=writeln!(out,"{}",value);let _=out.flush();
}

struct State {
    pet:PetWindow,settings:Option<SettingsWindow>,bubble:Option<BubbleWindow>,
    common:PathBuf,python:String,worker:Option<Arc<Mutex<Child>>>,progress:i32,
    rows:Vec<String>,page:i32,actions:i32,theme:String,saved:bool,moving:bool,frame:i32,ticks:i32,
    tick_intervals:Vec<f64>,last_tick:Instant,
}
type Shared=Rc<RefCell<State>>;
fn apply_theme(s:&State) {
    let dark=s.theme=="dark"||(s.theme=="system"&&native::dark_preference());
    s.pet.global::<Theme>().set_dark(dark);
    if let Some(ui)=&s.settings{ui.global::<Theme>().set_dark(dark);}
    if let Some(ui)=&s.bubble{ui.global::<Theme>().set_dark(dark);}
}

fn open_settings(state:&Shared)->Result<(),String> {
    if state.borrow().settings.is_some(){return Ok(())}
    let ui=SettingsWindow::new().map_err(|e|e.to_string())?;
    ui.window().set_rendering_notifier(|stage,_|if matches!(stage,slint::RenderingState::AfterRendering){frame_complete("settings")}).map_err(|e|format!("Render hook: {e:?}"))?;
    ui.set_page(state.borrow().page);ui.set_progress(state.borrow().progress);
    ui.set_theme_index(match state.borrow().theme.as_str(){"dark"=>2,"light"=>1,_=>0});
    ui.global::<Theme>().set_dark(state.borrow().theme=="dark"||(state.borrow().theme=="system"&&native::dark_preference()));
    let weak=Rc::downgrade(state);ui.on_show_message(move||{if let Some(s)=weak.upgrade(){let _=show_message(&s);}});
    let weak=Rc::downgrade(state);ui.on_start_load(move||{if let Some(s)=weak.upgrade(){let _=start_worker(&s);}});
    let weak=Rc::downgrade(state);ui.on_stop_load(move||{if let Some(s)=weak.upgrade(){stop_worker(&s);}});
    let weak=Rc::downgrade(state);ui.on_save(move||{if let Some(s)=weak.upgrade(){s.borrow_mut().saved=true;}});
    ui.on_quit(||{let _=slint::quit_event_loop();});
    let weak=Rc::downgrade(state);ui.on_theme_changed(move|index|{
        if let Some(s)=weak.upgrade(){let mut s=s.borrow_mut();s.theme=match index{2=>"dark",1=>"light",_=>"system"}.into();apply_theme(&s);}
    });
    let weak=Rc::downgrade(state);ui.window().on_close_requested(move||{
        if let Some(s)=weak.upgrade(){s.borrow_mut().settings=None;}
        slint::CloseRequestResponse::HideWindow
    });
    ui.window().set_position(slint::PhysicalPosition::new(240,80));
    ui.show().map_err(|e|e.to_string())?;state.borrow_mut().settings=Some(ui);Ok(())
}
fn close_settings(state:&Shared){if let Some(ui)=state.borrow_mut().settings.take(){let _=ui.hide();}}
fn show_message(state:&Shared)->Result<(),String> {
    if state.borrow().bubble.is_none(){
        let ui=BubbleWindow::new().map_err(|e|e.to_string())?;
        ui.global::<Theme>().set_dark(state.borrow().theme=="dark"||(state.borrow().theme=="system"&&native::dark_preference()));
        let common=state.borrow().common.clone();let weak=ui.as_weak();let mut applied=0;
        ui.window().set_rendering_notifier(move|stage,_|if matches!(stage,slint::RenderingState::AfterRendering){
            if let Some(ui)=weak.upgrade(){let hwnd=native::handle(ui.window()).unwrap_or(0);if hwnd!=0&&hwnd!=applied{if native::overlay(ui.window(),&common,false).is_ok(){applied=hwnd;}}}
            frame_complete("bubble")
        }).map_err(|e|format!("Render hook: {e:?}"))?;
        let weak=ui.as_weak();ui.on_confirmed(move||{if let Some(ui)=weak.upgrade(){let _=ui.hide();}});
        ui.window().set_position(slint::PhysicalPosition::new(40,280));state.borrow_mut().bubble=Some(ui);
    }
    let s=state.borrow();let ui=s.bubble.as_ref().unwrap();ui.show().map_err(|e|e.to_string())?;
    Ok(())
}
fn start_worker(state:&Shared)->Result<(),String> {
    {
        let s=state.borrow();if let Some(w)=&s.worker {if w.lock().unwrap().try_wait().map_err(|e|e.to_string())?.is_none(){return Ok(())}}
    }
    let s=state.borrow();
    let mut child=Command::new(&s.python).arg("-B").arg(s.common.join("load_worker.py"))
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).creation_flags(0x08000000).spawn().map_err(|e|e.to_string())?;
    let output=child.stdout.take().unwrap();let ui_weak=s.pet.as_weak();drop(s);
    let child=Arc::new(Mutex::new(child));state.borrow_mut().worker=Some(child);
    std::thread::spawn(move||{
        for line in std::io::BufReader::new(output).lines().map_while(Result::ok){
            if let Ok(v)=serde_json::from_str::<Value>(&line){if let Some(n)=v["value"].as_i64(){
                let weak=ui_weak.clone();let _=weak.upgrade_in_event_loop(move|_ui|{
                    UI_STATE.with(|slot|if let Some(s)=slot.borrow().as_ref(){let mut s=s.borrow_mut();s.progress=n as i32;if let Some(ui)=&s.settings{ui.set_progress(n as i32);}});
                });
            }}
        }
    });Ok(())
}
fn stop_worker(state:&Shared){
    if let Some(w)=&state.borrow().worker{let mut w=w.lock().unwrap();if let Some(stdin)=&mut w.stdin{let _=writeln!(stdin,"stop");let _=stdin.flush();}}
}
fn snapshot(window:&slint::Window,path:&Path)->Result<(),String>{
    let p=window.take_snapshot().map_err(|e|e.to_string())?;
    image::save_buffer(path,p.as_bytes(),p.width(),p.height(),image::ColorType::Rgba8).map_err(|e|e.to_string())
}
fn handle(state:&Shared,input:&Value)->Result<Value,String>{
    let n=|key:&str,default:i64|input[key].as_i64().unwrap_or(default);
    match input["command"].as_str().unwrap_or("") {
        "ping"=>{let s=state.borrow();Ok(json!({"actions":s.actions,"progress":s.progress,"visible":s.pet.window().is_visible()}))},
        "settings"=>{if n("show",1)==1{open_settings(state)?}else{close_settings(state)}Ok(json!({"visible":state.borrow().settings.is_some()}))},
        "page"=>{open_settings(state)?;let mut s=state.borrow_mut();s.page=n("page",0) as i32;s.settings.as_ref().unwrap().set_page(s.page);s.actions+=1;Ok(json!({"page":s.page}))},
        "theme"=>{
            let theme=input["theme"].as_str().unwrap_or("dark");let mut s=state.borrow_mut();s.theme=theme.into();
            apply_theme(&s);s.actions+=1;Ok(json!({"theme":theme}))
        },
        "message"=>{show_message(state)?;Ok(json!({"visible":true}))},
        "confirm"=>{let mut s=state.borrow_mut();if let Some(ui)=&s.bubble{ui.hide().map_err(|e|e.to_string())?}s.actions+=1;Ok(json!({"confirmed":true}))},
        "library"=>{
            {let mut s=state.borrow_mut();if s.rows.is_empty(){s.rows=(0..10000).map(|i|format!("{:05} · Фраза для перерыва: встань и немного пройдись.",i+1)).collect();}}
            open_settings(state)?;let mut s=state.borrow_mut();s.page=2;let ui=s.settings.as_ref().unwrap();ui.set_page(2);
            let selected:Vec<slint::SharedString>=s.rows.iter().skip(n("offset",0) as usize).take(50).map(|v|v.as_str().into()).collect();
            let count=selected.len();ui.set_rows(ModelRc::new(VecModel::from(selected)));Ok(json!({"total":s.rows.len(),"shown":count}))
        },
        "load"=>{start_worker(state)?;let s=state.borrow();Ok(json!({"pid":s.worker.as_ref().unwrap().lock().unwrap().id()}))},
        "cancel"=>{stop_worker(state);Ok(json!({"requested":true}))},
        "worker"=>{let s=state.borrow();if let Some(w)=&s.worker{let mut w=w.lock().unwrap();Ok(json!({"running":w.try_wait().map_err(|e|e.to_string())?.is_none(),"progress":s.progress,"pid":w.id()}))}else{Ok(json!({"running":false,"progress":0}))}},
        "animate"=>{let mut s=state.borrow_mut();s.moving=n("enabled",1)!=0;Ok(json!({"moving":s.moving}))},
        "native"=>Ok(native::inspect(state.borrow().pet.window())),
        "visibility"=>{
            let s=state.borrow();if n("show",1)==1{s.pet.show().map_err(|e|e.to_string())?;}else{s.pet.hide().map_err(|e|e.to_string())?}Ok(json!({"visible":s.pet.window().is_visible()}))
        },
        "snapshot"=>{
            let base=input["path"].as_str().ok_or("Missing path")?;let s=state.borrow();
            if let Some(p)=Path::new(base).parent(){std::fs::create_dir_all(p).map_err(|e|e.to_string())?}
            snapshot(s.pet.window(),Path::new(&format!("{base}-pet.png")))?;
            if let Some(ui)=&s.settings{snapshot(ui.window(),Path::new(&format!("{base}-settings.png")))?}
            if let Some(ui)=&s.bubble{if ui.window().is_visible(){snapshot(ui.window(),Path::new(&format!("{base}-bubble.png")))?}}
            Ok(json!({"saved":true}))
        },
        "stats"=>{let s=state.borrow();Ok(json!({"actions":s.actions,"timer_intervals_ms":s.tick_intervals,"rows":s.rows.len(),"saved":s.saved}))},
        "quit"=>{let _=slint::quit_event_loop();Ok(json!({"bye":true}))},
        other=>Err(format!("Unknown command: {other}"))
    }
}
thread_local! {static UI_STATE:RefCell<Option<Shared>>=const {RefCell::new(None)};}
fn main()->Result<(),Box<dyn std::error::Error>> {
    let started=Instant::now();let args:Vec<String>=std::env::args().collect();
    let arg=|key:&str,default:&str|args.iter().position(|x|x==key).and_then(|i|args.get(i+1)).cloned().unwrap_or(default.into());
    let common=PathBuf::from(arg("--common","common"));let bench=args.iter().any(|s|s=="--bench");
    let renderer=arg("--renderer","femtovg");
    slint::BackendSelector::new().backend_name("winit".into()).renderer_name(renderer.clone()).with_winit_window_attributes_hook(move|a|{
        let overlay=a.title.contains("маскот")||a.title.contains("сообщение");
        let a=if overlay{a.with_transparent(true).with_skip_taskbar(true)}else{a};
        a.with_active(!bench&&!overlay)
    }).select()?;
    let pet=PetWindow::new()?;pet.window().set_position(slint::PhysicalPosition::new(40,80));
    let state=Rc::new(RefCell::new(State{pet,settings:None,bubble:None,common,python:arg("--python","python"),worker:None,progress:0,
        rows:vec![],page:0,actions:0,theme:"system".into(),saved:false,moving:true,frame:0,ticks:0,tick_intervals:vec![],last_tick:Instant::now()}));
    UI_STATE.with(|s|*s.borrow_mut()=Some(state.clone()));
    let weak=Rc::downgrade(&state);state.borrow().pet.on_open_settings(move||{if let Some(s)=weak.upgrade(){let _=open_settings(&s);}});
    let foreground=native::foreground();
    state.borrow().pet.global::<Theme>().set_dark(native::dark_preference());
    let weak=Rc::downgrade(&state);let mut ready=false;let mut applied=0;
    state.borrow().pet.window().set_rendering_notifier(move|stage,_|{
        if !matches!(stage,slint::RenderingState::AfterRendering){return}
        if let Some(s)=weak.upgrade(){let s=s.borrow();let hwnd=native::handle(s.pet.window()).unwrap_or(0);
            let result=if hwnd!=0&&hwnd!=applied{let result=native::overlay(s.pet.window(),&s.common,true);if result.is_ok(){applied=hwnd;}result}else{Ok(())};
            frame_complete("pet");if ready{return}ready=true;
            emit(json!({"event_name":"ready","stack":"slint","pid":std::process::id(),"elapsed_ms":started.elapsed().as_secs_f64()*1000.,
                "foreground_unchanged":foreground==native::foreground(),"native":native::inspect(s.pet.window()),"overlay_error":result.err()}));}
    }).map_err(|e|format!("Rendering notifier: {e:?}"))?;
    state.borrow().pet.show()?;
    let tick=slint::Timer::default();let weak=Rc::downgrade(&state);
    tick.start(slint::TimerMode::Repeated,Duration::from_millis(33),move||{
        if let Some(state)=weak.upgrade(){let mut s=state.borrow_mut();let now=Instant::now();let interval=(now-s.last_tick).as_secs_f64()*1000.;s.last_tick=now;
            if s.tick_intervals.len()<100000{s.tick_intervals.push(interval)}
            if s.moving&&s.pet.window().is_visible(){s.frame+=1;s.pet.set_bounce((s.frame as f32/7.).sin()*3.);}
            s.ticks+=1;if s.ticks%30==0&&s.theme=="system"{apply_theme(&s);}
        }
    });
    if !bench {open_settings(&state)?;}
    if bench {let weak=state.borrow().pet.as_weak();std::thread::spawn(move||{
        for line in std::io::stdin().lock().lines().map_while(Result::ok){
            let Ok(input)=serde_json::from_str::<Value>(&line) else {continue};let received=Instant::now();let weak=weak.clone();
            let _=weak.upgrade_in_event_loop(move|_ui|UI_STATE.with(|slot|{
                if let Some(s)=slot.borrow().as_ref(){
                    let id=&input["id"];
                    if input["command"]=="present" {
                        let target=input["target"].as_str().unwrap_or("settings");
                        FRAME_WAITERS.with(|w|w.borrow_mut().push((id.clone(),received,target.into())));
                        let state=s.borrow();
                        match target {"pet"=>state.pet.window().request_redraw(),"bubble"=>{if let Some(ui)=&state.bubble{ui.window().request_redraw();}},_=>{if let Some(ui)=&state.settings{ui.window().request_redraw();}}}
                        return;
                    }
                    let queue_ms=received.elapsed().as_secs_f64()*1000.;match handle(s,&input){
                    Ok(result)=>emit(json!({"id":id,"ok":true,"queue_ms":queue_ms,"result":result})),
                    Err(error)=>emit(json!({"id":id,"ok":false,"error":error}))
                }}
            }));
        }
        let _=slint::quit_event_loop();
    });}
    slint::run_event_loop_until_quit()?;
    tick.stop();let mut s=state.borrow_mut();if let Some(w)=&s.worker{let mut w=w.lock().unwrap();let _=w.kill();let _=w.wait();}s.settings=None;s.bubble=None;
    Ok(())
}
