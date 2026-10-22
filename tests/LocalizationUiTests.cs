using System;
using System.Collections.Generic;
using System.Drawing;
using System.IO;
using System.Linq;
using System.Reflection;
using System.Text.RegularExpressions;
using System.Web.Script.Serialization;
using System.Windows.Forms;

public static class LocalizationUiTests {
 static readonly BindingFlags Private=BindingFlags.Instance|BindingFlags.NonPublic;
 static int passed;
 static T Get<T>(object owner,string name){return (T)owner.GetType().GetField(name,Private).GetValue(owner);}
 static void Call(object owner,string name,params object[] args){owner.GetType().GetMethod(name,Private).Invoke(owner,args);}
 static IEnumerable<Control> All(Control root){yield return root;foreach(Control child in root.Controls)foreach(var descendant in All(child))yield return descendant;}
 static void Check(bool yes,string name){if(!yes)throw new Exception(name);Console.WriteLine("PASS "+name);passed++;}
 static void Close(MainForm form){Call(form,"CloseLocalAi");Get<NotifyIcon>(form,"tray").Dispose();Get<Timer>(form,"timer").Dispose();form.Dispose();}
 [STAThread]public static int Main(string[] args){int at=Array.IndexOf(args,"--data-dir");if(at<0||!args.Contains("--self-test"))return 2;string root=Path.GetFullPath(args[at+1]);Directory.CreateDirectory(root);try{
  Application.EnableVisualStyles();Application.SetCompatibleTextRenderingDefault(false);
  foreach(string language in new[]{"ru","en"}){
   var settings=new Settings{UiLanguage=language,SpeechLanguage="ru",SetupComplete=true,TextGeneration=false,VoiceProvider="System",WorkStart=540,WorkEnd=1020,BreakEvery=45,ExerciseAfter=90};
   SettingsFile.Write(Path.Combine(root,"settings.json"),new JavaScriptSerializer().Serialize(settings));
   var form=new MainForm();try{
    form.Show();Call(form,"UpdateLabels");var controls=All(form).ToArray();
    if(language=="en"){
     var untranslated=controls.Where(control=>(control is Label||control is ButtonBase||control is TabPage)&&Regex.IsMatch(control.Text,"[А-Яа-яЁё]")).Select(control=>control.Text).ToArray();
     Check(untranslated.Length==0,"English advanced captions have no untranslated Russian: "+string.Join(" | ",untranslated));
     Check(Get<ComboBox>(form,"textChoice").Items.Cast<object>().All(item=>!Regex.IsMatch(item.ToString(),"[А-Яа-яЁё]"))&&Get<CheckedListBox>(form,"categoryChoice").Items.Cast<object>().All(item=>!Regex.IsMatch(item.ToString(),"[А-Яа-яЁё]")),"English model and category choices use the active language");
     Check(Get<TextBox>(form,"characterDescription").Text.StartsWith("You are Kavychka"),"English default personality is shown in the editor");
    }
    Check(Get<Label>(form,"schedulePeriod").Text.Contains("45")&&Get<Label>(form,"schedulePeriod").Text.Contains("09:00")&&Get<Label>(form,"exerciseHint").Text.Contains("90"),"advanced overview reflects the saved custom schedule "+language);
    string custom="Неизменяемый пользовательский характер.";Get<TextBox>(form,"characterDescription").Text=custom;
    var catalog=Get<List<Dictionary<string,object>>>(form,"deliveryCatalog");catalog.Clear();catalog.Add(new Dictionary<string,object>{{"id","confident"},{"name","Уверенно"},{"instruction","Speak confidently."}});catalog.Add(new Dictionary<string,object>{{"id","usercustom"},{"name","Моя подача"},{"instruction","My instruction"}});
    Get<Settings>(form,"settings").UiLanguage=language=="ru"?"en":"ru";L.Language=Get<Settings>(form,"settings").UiLanguage;Call(form,"RefreshAdvancedLocalization");
    Check(Get<TextBox>(form,"characterDescription").Text==custom,"language switching preserves unsaved custom personality "+language);
    var choices=Get<ComboBox>(form,"deliveryChoice").Items.Cast<object>().Select(item=>item.ToString()).ToArray();
    Check(choices.Contains("Моя подача")&&choices.Contains(L.Language=="en"?"Confident":"Уверенно"),"only built-in delivery names are localized "+language);
    Check(Get<ComboBox>(form,"textChoice").Items[0].ToString().Contains(L.Language=="en"?"6.98 GB":"6,98 ГБ"),"model choices refresh after a language change "+language);
    Check(Get<ComboBox>(form,"ttsChoice").Items[0].ToString().Contains(L.Language=="en"?"delivery control":"интонации"),"speech model choices refresh after a language change "+language);
    Check(Get<Settings>(form,"settings").SpeechLanguage=="ru","UI language does not change the reminder language "+language);
   }finally{Close(form);}
  }
  L.Language="en";using(var editor=new PresetEditorForm(root,AppDomain.CurrentDomain.BaseDirectory,"character","")){Check(Get<TextBox>(editor,"description").Text.StartsWith("You are Kavychka"),"default preset editor has an English personality description");}
  Console.WriteLine("PASS "+passed+" localization UI checks");return 0;
 }catch(Exception error){Console.Error.WriteLine(error);return 1;}}
}
