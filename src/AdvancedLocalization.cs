using System;
using System.Linq;
using System.Windows.Forms;

public partial class MainForm {
 bool refreshingAdvanced;
 string advancedLanguage="";
 static void Refill(ComboBox choice,object[] names){int index=choice.SelectedIndex;choice.Items.Clear();choice.Items.AddRange(names);choice.SelectedIndex=Math.Min(names.Length-1,Math.Max(0,index));}
 static void TranslateCaptions(Control root){if(root is ButtonBase||root is Label||root is TabPage||root is Form)root.Text=AppText.Caption(root.Text);var list=root as ListView;if(list!=null)foreach(ColumnHeader column in list.Columns)column.Text=AppText.Caption(column.Text);foreach(Control child in root.Controls)TranslateCaptions(child);}
 string DeliveryDisplayName(System.Collections.Generic.Dictionary<string,object> item){
  string id=S(item,"id"),name=S(item,"name");string[] ids={"confident","calm","amused"},ru={"Уверенно","Спокойно","С лёгкой усмешкой"},en={"Confident","Calm","Slightly amused"};int index=Array.IndexOf(ids,id);
  return index>=0&&(name==ru[index]||name==en[index])?L.T(ru[index],en[index]):name;
 }
 void RefreshAdvancedLocalization(){
  if(advancedLanguage==L.Language)return;refreshingAdvanced=true;bool wasLoading=localLoading;localLoading=true;
  try{
   TranslateCaptions(this);
   Refill(textChoice,TextModelNames);Refill(runtimeTextChoice,TextModelNames);
   Refill(ttsChoice,new object[]{L.T("CosyVoice 3 0.5B · интонации · 5,43 ГБ","CosyVoice 3 0.5B · delivery control · 5.43 GB"),L.T("Qwen3-TTS 0.6B · 2,52 ГБ","Qwen3-TTS 0.6B · 2.52 GB"),L.T("Qwen3-TTS 1.7B · 4,54 ГБ","Qwen3-TTS 1.7B · 4.54 GB")});
   Refill(deviceChoice,new object[]{L.T("Автоматически · NVIDIA при наличии CUDA","Automatic · NVIDIA when CUDA is available"),L.T("CPU · медленнее, без видеокарты","CPU · slower, no graphics card needed"),"NVIDIA CUDA"});
   for(int i=0;i<categoryChoice.Items.Count;i++){var state=categoryChoice.GetItemCheckState(i);categoryChoice.Items[i]=CategoryNames[i];categoryChoice.SetItemCheckState(i,state);}
   string name=deliveryName.Text,instruction=deliveryInstruction.Text,id=DeliveryIds[Math.Max(0,deliveryChoice.SelectedIndex)];PopulateDeliveries(id);deliveryName.Text=name;deliveryInstruction.Text=instruction;
   string nextDefault=CharacterLines(PersonalPresets.DefaultDescription(Application.StartupPath,L.Language));
   if(settings.CharacterFile.Length==0&&characterDescription.Text==defaultCharacter)characterDescription.Text=nextDefault;defaultCharacter=nextDefault;
   foreach(string key in componentStates.Keys.ToArray()){componentStates[key]=AppText.Parts(componentStates[key]);if(componentLabels.ContainsKey(key))componentLabels[key].Text=componentStates[key];}
   foreach(ListViewItem row in library.Items){if(row.SubItems.Count<6)continue;row.SubItems[1].Text=AppText.Caption(row.SubItems[1].Text);row.SubItems[4].Text=AppText.Caption(row.SubItems[4].Text);}
   advancedLanguage=L.Language;UpdateLabels();
  }finally{localLoading=wasLoading;refreshingAdvanced=false;}
 }
}
