using System;
using System.Collections.Generic;
using System.Drawing;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Windows.Forms;
using System.Web.Script.Serialization;

// Exercises real settings bindings using an in-memory save callback. No models,
// hooks, speech, backend or user data are opened by this test executable.
public static class V2UiTests {
 static int passed;
 static void Check(bool value,string name){if(!value)throw new Exception(name);Console.WriteLine("PASS "+name);passed++;}
 static IEnumerable<Control> All(Control root){foreach(Control child in root.Controls){yield return child;foreach(var descendant in All(child))yield return descendant;}}
 static T Field<T>(object value,string name){return (T)value.GetType().GetField(name,BindingFlags.Instance|BindingFlags.NonPublic).GetValue(value);}
 [STAThread] public static int Main(){try{
  Application.EnableVisualStyles();Application.SetCompatibleTextRenderingDefault(false);
  bool windowsDark=false;var realReader=StudioColors.SystemDarkReader;StudioColors.SystemDarkReader=delegate{return windowsDark;};
  foreach(string language in new[]{"ru","en"}){
   L.Language=language;var original=new Settings{UiLanguage=language,SpeechLanguage=language,TimeZoneId="UTC",SetupComplete=true};
   Settings saved=null;
   using(var form=new SimpleSettingsForm(original,new List<Dictionary<string,object>>(),false)){
    form.Saved=delegate(Settings copy){saved=copy;};form.Show();
    var fields=Field<Dictionary<string,Control>>(form,"fields");
    Check(original.ThemeMode=="system"&&!StudioColors.IsDark,"defaults follow Windows light preference "+language);
    string persistedMode="";form.ThemeChanged=delegate(string mode){persistedMode=mode;original.ThemeMode=mode;};
    using(var bubble=new PetMessage("Take a break.","Stand up","Stop voice")){
     bubble.Show();var theme=(ComboBox)fields["ThemeMode"];
     theme.SelectedIndex=2;
     Check(StudioColors.IsDark&&persistedMode=="dark"&&form.BackColor==StudioColors.Background&&bubble.BackColor==StudioColors.Background,"explicit dark theme applies to settings and existing messages "+language);
     Check(fields["UserName"].BackColor==StudioColors.Surface&&fields["UserName"].ForeColor==StudioColors.Ink,"hidden page inputs receive the theme "+language);
     theme.SelectedIndex=1;Check(!StudioColors.IsDark&&persistedMode=="light"&&bubble.BackColor==StudioColors.Background,"switch back to light immediately "+language);
     windowsDark=true;theme.SelectedIndex=0;Check(StudioColors.IsDark&&persistedMode=="system","system mode reads current Windows preference "+language);
     windowsDark=false;StudioColors.RefreshSystem();Check(!StudioColors.IsDark&&form.BackColor==StudioColors.Background,"system preference changes recolor open forms "+language);
     theme.SelectedIndex=2;StudioColors.RefreshSystem();Check(StudioColors.IsDark,"manual selection ignores Windows changes "+language);
     bubble.Close();
    }
    fields["UserName"].Text="Sample user";((StudioClock)fields["WorkStart"]).Minutes=570;((CheckBox)fields["RunAtLogin"]).Checked=true;
    ((NumericUpDown)fields["BreakEvery"]).Value=45;((CheckBox)fields["TrustConfirmation"]).Checked=true;
    ((ComboBox)fields["TextChoice"]).SelectedIndex=1;((ComboBox)fields["VoiceChoice"]).SelectedIndex=2;
    form.SelectPage(1);var days=(StudioDays)fields["Days"];((StudioButton)days.Controls[5]).PerformClick();
    Check(original.UserName==""&&original.WorkStart==600&&!original.TrustConfirmation,"editing does not change live settings "+language);
    bool ok=(bool)typeof(SimpleSettingsForm).GetMethod("SaveDraft",BindingFlags.Instance|BindingFlags.NonPublic).Invoke(form,new object[]{true});
    Check(ok&&saved!=null&&saved.UserName=="Sample user"&&saved.WorkStart==570&&saved.BreakEvery==45&&saved.TrustConfirmation,"settings save round trip "+language);
    Check(saved.WorkDays.Contains(6)&&saved.WorkDays.Length==6,"weekday buttons retain chosen days "+language);
    Check(!original.RunAtLogin&&saved.RunAtLogin,"startup choice is explicit and saved with the draft "+language);
    Check(saved.TextGeneration&&saved.TextModel=="text-gemma4-12b"&&saved.VoiceModel=="tts-0.6b","simple model selectors map to backend IDs "+language);
    form.UpdateHardware(new Dictionary<string,object>{{"ram_bytes",8000000000L},{"vram_bytes",6000000000L},{"gpu_name","Test NVIDIA"}},new Dictionary<string,object>{{"text_gpu",true},{"speech_gpu",true}});
    var hardwareLabel=Field<Label>(form,"hardwareInfo");Check(hardwareLabel.Text.Contains("Test NVIDIA")&&hardwareLabel.Text.Contains(language=="ru"?"может не хватить":"may exceed VRAM"),"large model shows detected memory and VRAM guidance "+language);
    ((ComboBox)fields["TextChoice"]).SelectedIndex=3;
    Check(!hardwareLabel.Text.Contains(language=="ru"?"может не хватить":"may exceed VRAM"),"choosing a smaller text model updates memory guidance immediately "+language);
    form.UpdateHardware(new Dictionary<string,object>{{"ram_bytes",8000000000L}},new Dictionary<string,object>{{"text_gpu",false},{"speech_gpu",false}});
    Check(hardwareLabel.Text.Contains(language=="ru"?"Голос: CPU":"Speech: CPU")&&hardwareLabel.Text.Contains(language=="ru"?"много времени":"long time"),"CPU speech explains slower preparation "+language);
    var json=new JavaScriptSerializer();Check(json.Deserialize<Settings>(json.Serialize(saved)).ThemeMode=="dark","theme survives settings serialization "+language);
    form.Size=form.MinimumSize;
    for(int page=0;page<5;page++){form.SelectPage(page);form.PerformLayout();Check(!All(form).OfType<FlowLayoutPanel>().Any(p=>p.Visible&&p.HorizontalScroll.Visible),"no horizontal scrolling at minimum size "+language+" page "+page);}
    form.SelectPage(4);form.UpdateStatus("Next event","Downloading 42%",true,42,false,"E:\\Sample");
    Check(!Field<StudioButton>(form,"prepareButton").Enabled&&Field<StudioButton>(form,"cancelButton").Visible&&Field<ProgressBar>(form,"modelProgress").Value==42,"visible preparation progress and cancel "+language);
    var cancelSetup=Field<StudioButton>(form,"cancelButton");var fixedActions=cancelSetup.Parent;
    Check(fixedActions.Dock==DockStyle.Bottom&&fixedActions.Visible&&fixedActions.Parent.ClientRectangle.Contains(fixedActions.Bounds)&&fixedActions.ClientRectangle.Contains(cancelSetup.Bounds)&&!All(form).OfType<NightScroll>().Any(scroll=>scroll.Contains(cancelSetup)),"preparation, progress and cancel remain outside the scrolling page at minimum size "+language);
    var modelScroll=All(form).OfType<NightScroll>().Single(scroll=>scroll.Visible);var rail=Field<Control>(modelScroll,"rail");
    Check(rail.Visible&&modelScroll.Controls.GetChildIndex(rail)==0,"scroll handle stays above wide page content "+language);
    typeof(Control).GetMethod("OnMouseDown",BindingFlags.Instance|BindingFlags.NonPublic).Invoke(rail,new object[]{new MouseEventArgs(MouseButtons.Left,1,7,rail.Height/2,0)});
    typeof(Control).GetMethod("OnMouseUp",BindingFlags.Instance|BindingFlags.NonPublic).Invoke(rail,new object[]{new MouseEventArgs(MouseButtons.Left,1,7,rail.Height/2,0)});
    Check(Field<int>(modelScroll,"offset")>0&&fixedActions.Visible&&fixedActions.ClientRectangle.Contains(cancelSetup.Bounds),"scroll handle moves page content without moving preparation controls "+language);modelScroll.ScrollTo(0);
    form.UpdateStatus("Next event","Ready",false,100,false,"E:\\Sample");Check(Field<StudioButton>(form,"prepareButton").Enabled&&!Field<StudioButton>(form,"cancelButton").Visible,"preparation returns to idle "+language);
    fields["UserName"].Text="Unsaved name";form.ReloadPresets(new List<Dictionary<string,object>>{new Dictionary<string,object>{{"id","samplevoice"},{"kind","voice"},{"name","Imported voice"}}},"voice","samplevoice");Check(fields["UserName"].Text=="Unsaved name"&&((ComboBox)fields["VoiceProfile"]).SelectedItem.ToString()=="Imported voice","preset import preserves other unsaved settings "+language);
    form.Close();
   }
   using(var bubble=new PetMessage(string.Join(" ",Enumerable.Repeat("Take a little break.",22)),"I am getting up","Stop voice")){bubble.Show();Check(bubble.Height>=200&&bubble.Height<=500,"message bubble grows for long text "+language);bubble.Close();}
  }
  L.Language="en";
  var beforeMigration=new Settings{TextGeneration=true,VoiceProvider="Qwen local",TimeZoneId="UTC"};
  using(var form=new SimpleSettingsForm(beforeMigration,new List<Dictionary<string,object>>(),false)){
   var fields=Field<Dictionary<string,Control>>(form,"fields");fields["UserName"].Text="Keep this draft";
   var migrated=new Settings{TextGeneration=true,VoiceProvider="Qwen local",TextModel="text-qwen35-4b",RuntimeTextModel="text-qvikhr3-8b",VoiceModel="tts-0.6b",RuntimeGeneration=true,Swear=true};
   form.RebaseModelChoices(migrated);
   Check(((ComboBox)fields["TextChoice"]).SelectedIndex==6&&((ComboBox)fields["VoiceChoice"]).SelectedIndex==2&&((CheckBox)fields["RuntimeGeneration"]).Checked&&((CheckBox)fields["Swear"]).Checked,"open settings receive migrated model choices without a stale save");
   Check(fields["UserName"].Text=="Keep this draft","migration preserves unrelated unsaved input");
   migrated.CharacterFile="migratedpersona";var presets=new List<Dictionary<string,object>>{new Dictionary<string,object>{{"id","migratedpersona"},{"kind","character"},{"name","Previous character"}},new Dictionary<string,object>{{"id","newpersona"},{"kind","character"},{"name","New character"}}};form.RebaseCharacterChoice(migrated,presets);
   Check(((ComboBox)fields["CharacterFile"]).SelectedItem.ToString()=="Previous character","open settings show the migrated character instead of resetting it on save");
   ((ComboBox)fields["CharacterFile"]).SelectedIndex=2;form.RebaseCharacterChoice(migrated,presets);Check(((ComboBox)fields["CharacterFile"]).SelectedItem.ToString()=="New character","migration preserves an unsaved character selection");
   ((ComboBox)fields["TextChoice"]).SelectedIndex=2;((ComboBox)fields["VoiceChoice"]).SelectedIndex=3;((CheckBox)fields["RuntimeGeneration"]).Checked=false;
   migrated.TextModel="text-saiga-llama3-8b";migrated.VoiceModel="tts-cosyvoice3";form.RebaseModelChoices(migrated);
   Check(((ComboBox)fields["TextChoice"]).SelectedIndex==2&&((ComboBox)fields["VoiceChoice"]).SelectedIndex==3&&!((CheckBox)fields["RuntimeGeneration"]).Checked,"migration never replaces unsaved model choices");
  }
  string fixture=Path.GetFullPath(Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"..","..","work","ui-presets-"+Guid.NewGuid().ToString("N")));Directory.CreateDirectory(fixture);
  try{
   foreach(string language in new[]{"ru","en"}){L.Language=language;
    using(var editor=new PresetEditorForm(fixture,AppDomain.CurrentDomain.BaseDirectory,"character","")){editor.Show();Field<TextBox>(editor,"name").Text="User character";Field<TextBox>(editor,"description").Text="Independent custom character";Check(Field<TextBox>(editor,"description").Height>=290,"character editor has usable multiline input "+language);typeof(PresetEditorForm).GetMethod("SavePreset",BindingFlags.Instance|BindingFlags.NonPublic).Invoke(editor,null);Check(editor.DialogResult==DialogResult.OK&&PersonalPresets.Description(fixture,editor.ResultId)=="Independent custom character","default character saves as a separate user preset "+language);}
    using(var editor=new PresetEditorForm(fixture,AppDomain.CurrentDomain.BaseDirectory,"voice","")){editor.Show();Field<TextBox>(editor,"name").Text="User voice";Field<TextBox>(editor,"transcript").Text="Exact reference words";typeof(PresetEditorForm).GetMethod("SavePreset",BindingFlags.Instance|BindingFlags.NonPublic).Invoke(editor,null);Check(editor.DialogResult==DialogResult.OK&&PersonalPresets.Transcripts(fixture,editor.ResultId)[0]=="Exact reference words","voice editor saves transcript in a user copy "+language);}
    using(var editor=new PresetEditorForm(fixture,AppDomain.CurrentDomain.BaseDirectory,"visual","")){editor.Show();Check(!Field<StudioButton>(editor,"save").Enabled&&All(editor).OfType<PictureBox>().Single().Image!=null,"default mascot preview is available "+language);editor.Close();}
   }
   string atlas=Path.Combine(fixture,"atlas.png");using(var image=new Bitmap(1536,1872)){using(var g=Graphics.FromImage(image)){g.Clear(Color.Transparent);g.FillRectangle(Brushes.Lime,0,0,96,208);g.FillRectangle(Brushes.Blue,96,0,96,208);}image.Save(atlas);}
   using(var pet=new PetForm()){pet.SetVisual(atlas);pet.CreateControl();using(var bitmap=new Bitmap(pet.Width,pet.Height)){pet.DrawToBitmap(bitmap,new Rectangle(0,0,bitmap.Width,bitmap.Height));int green=0,blue=0;for(int y=0;y<bitmap.Height;y++)for(int x=0;x<bitmap.Width;x++){var color=bitmap.GetPixel(x,y);if(color.G>200&&color.R<30&&color.B<30)green++;if(color.B>200&&color.R<30&&color.G<30)blue++;}Check(green>100&&blue>100,"Codex atlas renderer displays the whole 192 pixel cell");}}
   Check(SpriteAtlas.Frame(4000,false,false,false,false).Width==192&&SpriteAtlas.Frame(800,false,true,false,false).Y==624&&SpriteAtlas.Frame(800,false,false,false,true).Y==832,"Codex atlas state geometry matches its public contract");
  }finally{string work=Path.GetFullPath(Path.Combine(AppDomain.CurrentDomain.BaseDirectory,"..","..","work"));if(fixture.StartsWith(work+Path.DirectorySeparatorChar,StringComparison.OrdinalIgnoreCase)&&Path.GetFileName(fixture).StartsWith("ui-presets-",StringComparison.Ordinal))Directory.Delete(fixture,true);}
  StudioColors.SystemDarkReader=realReader;StudioColors.SetMode("system");
  Console.WriteLine("PASS "+passed+" UI checks");return 0;
 }catch(Exception error){Console.Error.WriteLine(error);return 1;}}
}
