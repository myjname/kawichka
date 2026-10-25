use std::{ffi::c_void, path::Path};
use serde_json::{Value, json};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

type H = *mut c_void;
#[repr(C)] struct StylePair {old:u32,new:u32}
type Subclass=unsafe extern "system" fn(H,u32,usize,isize,usize,usize)->isize;
#[link(name="comctl32")]
unsafe extern "system" {
    fn SetWindowSubclass(h:H,proc:Subclass,id:usize,data:usize)->i32;
    fn RemoveWindowSubclass(h:H,proc:Subclass,id:usize)->i32;
    fn DefSubclassProc(h:H,message:u32,wp:usize,lp:isize)->isize;
}
unsafe extern "system" fn overlay_message(h:H,message:u32,wp:usize,lp:isize,id:usize,_data:usize)->isize {
    unsafe {
        if message==0x21{return 3}
        if message==0x82{RemoveWindowSubclass(h,overlay_message,id);}
        let result=DefSubclassProc(h,message,wp,lp);
        if message==0x7c&&wp as isize == -20 {let s=&mut *(lp as *mut StylePair);s.new=(s.new|0x08000080)&!0x00040000;}
        result
    }
}
#[repr(C)] #[derive(Default)] struct Rect { left:i32, top:i32, right:i32, bottom:i32 }
#[link(name="user32")]
unsafe extern "system" {
    fn GetForegroundWindow()->H;
    fn GetWindowLongPtrW(h:H,index:i32)->isize;
    fn SetWindowLongPtrW(h:H,index:i32,value:isize)->isize;
    fn SetWindowPos(h:H,after:H,x:i32,y:i32,w:i32,height:i32,flags:u32)->i32;
    fn GetClientRect(h:H,rect:*mut Rect)->i32;
    fn GetDpiForWindow(h:H)->u32;
    fn SetWindowRgn(h:H,region:H,redraw:i32)->i32;
    fn GetWindowRgn(h:H,region:H)->i32;
}
#[link(name="gdi32")]
unsafe extern "system" {
    fn CreateRectRgn(x:i32,y:i32,right:i32,bottom:i32)->H;
    fn CombineRgn(output:H,a:H,b:H,mode:i32)->i32;
    fn DeleteObject(h:H)->i32;
    fn PtInRegion(h:H,x:i32,y:i32)->i32;
}
pub fn foreground()->usize { unsafe { GetForegroundWindow() as usize } }
#[link(name="advapi32")]
unsafe extern "system" { fn RegGetValueW(key:H,subkey:*const u16,value:*const u16,flags:u32,kind:*mut u32,data:*mut c_void,size:*mut u32)->i32; }
pub fn dark_preference()->bool {
    let key:Vec<u16>="Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0".encode_utf16().collect();
    let name:Vec<u16>="AppsUseLightTheme\0".encode_utf16().collect();let mut value=1u32;let mut size=4u32;
    unsafe {RegGetValueW(0x80000001u32 as i32 as isize as H,key.as_ptr(),name.as_ptr(),16,std::ptr::null_mut(),&mut value as *mut _ as _,&mut size)==0&&value==0}
}
pub fn handle(window:&slint::Window)->Option<usize> {
    let h=window.window_handle();
    match h.window_handle().ok()?.as_raw() {RawWindowHandle::Win32(h)=>Some(h.hwnd.get() as usize),_=>None}
}
pub fn overlay(window:&slint::Window,common:&Path,mascot:bool)->Result<(),String> {
    let h=handle(window).ok_or("Window has no HWND")? as H;
    unsafe {
        if SetWindowSubclass(h,overlay_message,0x4b5651,0)==0{return Err("SetWindowSubclass failed".into())}
        let style=GetWindowLongPtrW(h,-20);
        SetWindowLongPtrW(h,-20,(style|0x08000080)&!0x00040000);
        SetWindowPos(h,-1isize as H,0,0,0,0,0x0010|0x0020|0x0001|0x0002);
        if !mascot {return Ok(())}
        let mask:Value=serde_json::from_str(&std::fs::read_to_string(common.join("mask.json")).map_err(|e|e.to_string())?).map_err(|e|e.to_string())?;
        let mut size=Rect::default(); GetClientRect(h,&mut size);
        let sx=size.right as f64/150.0;let sy=size.bottom as f64/170.0;
        let region=CreateRectRgn(0,0,0,0);
        for r in mask["rects"].as_array().ok_or("Invalid mask")? {
            let part=CreateRectRgn((r[0].as_f64().unwrap()*sx).floor() as i32,(r[1].as_f64().unwrap()*sy).floor() as i32,
                (r[2].as_f64().unwrap()*sx).ceil() as i32,(r[3].as_f64().unwrap()*sy).ceil() as i32);
            CombineRgn(region,region,part,2);DeleteObject(part);
        }
        if SetWindowRgn(h,region,1)==0 {DeleteObject(region);return Err("SetWindowRgn failed".into())}
    }
    Ok(())
}
pub fn inspect(window:&slint::Window)->Value {
    let Some(h)=handle(window) else {return json!({"error":"No HWND"})};let h=h as H;
    unsafe {
        let style=GetWindowLongPtrW(h,-20);let mut size=Rect::default();GetClientRect(h,&mut size);
        let r=CreateRectRgn(0,0,0,0);let has=GetWindowRgn(h,r)>0;
        let out=json!({"hwnd":h as usize,"no_activate":style&0x08000000!=0,"tool_window":style&0x80!=0,
            "topmost":style&8!=0,"dpi":GetDpiForWindow(h),"width":size.right,"height":size.bottom,
            "region_present":has,"transparent_corner_excluded":has&&PtInRegion(r,1,1)==0,
            "solid_part_included":has&&PtInRegion(r,size.right*42/150,size.bottom*60/170)!=0});
        DeleteObject(r);out
    }
}
