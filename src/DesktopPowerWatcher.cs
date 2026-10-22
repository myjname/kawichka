using System;
using System.Windows.Forms;
using Microsoft.Win32;

public sealed class DesktopPowerWatcher:IDisposable {
 readonly Control dispatcher;
 readonly Action<PowerModes,DateTime> changed;
 bool disposed;
 public DesktopPowerWatcher(Control dispatcher,Action<PowerModes,DateTime> changed){this.dispatcher=dispatcher;this.changed=changed;SystemEvents.PowerModeChanged+=Changed;}
 void Changed(object sender,PowerModeChangedEventArgs args){
  // Capture notification time before queueing: the UI may only process the
  // callback after the computer has resumed.
  DateTime observed=DateTime.UtcNow;
  if(disposed||dispatcher.IsDisposed||!dispatcher.IsHandleCreated)return;
  try{dispatcher.BeginInvoke((Action)delegate{if(!disposed&&!dispatcher.IsDisposed)changed(args.Mode,observed);});}catch(InvalidOperationException){}
 }
 public void Dispose(){if(disposed)return;disposed=true;SystemEvents.PowerModeChanged-=Changed;}
}
