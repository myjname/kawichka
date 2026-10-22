using System;
using System.Collections.Generic;
using System.Drawing;
using System.IO;
using System.Linq;
using System.Media;
using System.Threading;
using System.Threading.Tasks;
using System.Web.Script.Serialization;
using System.Windows.Forms;

public sealed class PresetEditorForm:NightForm {
 readonly string root,app,kind,original;
 readonly TextBox name,description,transcript;
 readonly Label feedback;
 readonly CancellationTokenSource cancel=new CancellationTokenSource();
 readonly FlowLayoutPanel page;
 readonly StudioButton save,export,delete;
 ComboBox references;
 ComboBox layout;
 string[] transcripts;
 int referenceIndex;
 SoundPlayer player;
 Image visual;
 bool busy,resourcesDisposed,closeWhenReady;
 public string ResultId;
 public bool Deleted;
 public PresetEditorForm(string root,string app,string kind,string id){
  this.root=root;this.app=app;this.kind=kind;original=id;ResultId=id;
  Text=kind=="voice"?L.T("Голос · просмотр и настройка","Voice · preview and settings"):kind=="character"?L.T("Характер · просмотр и настройка","Personality · preview and settings"):L.T("Внешность · просмотр и настройка","Appearance · preview and settings");
  ClientSize=new Size(650,720);MinimumSize=new Size(650,620);BackColor=StudioColors.Background;ForeColor=StudioColors.Ink;Font=new Font("Segoe UI",10);StartPosition=FormStartPosition.CenterParent;
  var scroll=new NightScroll{Dock=DockStyle.Fill};Controls.Add(scroll);page=new FlowLayoutPanel{Width=620,MinimumSize=new Size(620,0),MaximumSize=new Size(620,0),AutoSize=true,AutoSizeMode=AutoSizeMode.GrowAndShrink,FlowDirection=FlowDirection.TopDown,WrapContents=false,Padding=new Padding(28,18,20,24),BackColor=BackColor,ForeColor=ForeColor};scroll.Controls.Add(page);
  Dictionary<string,object> item;
  if(id.Length>0)item=PersonalPresets.Read(root,id,kind);else if(kind=="character")item=ReadDefault(Path.Combine(app,"assets","character.json"));else if(kind=="voice")item=ReadDefault(Path.Combine(app,"assets","voice","profile.json"));else item=new Dictionary<string,object>();
  Label(L.T("Название пресета","Preset name"));name=Entry(id.Length>0?Convert.ToString(item["name"]):L.T("Моя Кавычка","My Kavychka"),80,32);name.ReadOnly=id.Length==0&&kind=="visual";
  if(kind=="character"){
   Label(L.T("Характер, отношение к тебе и манера речи","Personality, attitude and speaking manner"));description=Entry(id.Length==0?PersonalPresets.DefaultDescription(app,L.Language):Convert.ToString(item["description"]),4000,300);description.Multiline=true;description.AcceptsReturn=true;description.ScrollBars=ScrollBars.Vertical;
   Hint(L.T("Описание влияет на новые фразы. Готовые тексты и записи остаются в библиотеке. Изменение стандартного характера создаёт отдельный пользовательский пресет.","The description affects new phrases. Existing text and audio stay in the library. Editing the default character creates a separate user preset."));
  }else if(kind=="voice"){
   transcripts=id.Length>0?PersonalPresets.Transcripts(root,id):new[]{item.ContainsKey("transcript")?Convert.ToString(item["transcript"]):""};
   Label(L.T("Записи голоса","Voice references"));references=new ComboBox{Width=550,DropDownStyle=ComboBoxStyle.DropDownList,DrawMode=DrawMode.OwnerDrawFixed,ItemHeight=26,BackColor=StudioColors.Surface,ForeColor=ForeColor,FlatStyle=FlatStyle.Flat};references.DrawItem+=delegate(object sender,DrawItemEventArgs e){e.Graphics.Clear(references.BackColor);if(e.Index>=0)TextRenderer.DrawText(e.Graphics,references.GetItemText(references.Items[e.Index]),Font,e.Bounds,StudioColors.Ink,TextFormatFlags.Left|TextFormatFlags.VerticalCenter);};for(int i=0;i<transcripts.Length;i++)references.Items.Add(L.T("Запись ","Reference ")+(i+1));references.SelectedIndex=0;page.Controls.Add(references);
   var row=Row();Button(row,L.T("Послушать запись","Play reference"),async delegate{await PreviewVoice();},245);Button(row,L.T("Остановить","Stop"),delegate{Stop();},245);
   Label(L.T("Точная расшифровка этой записи (необязательно)","Exact transcript of this reference (optional)"));transcript=Entry(transcripts[0],4000,135);transcript.Multiline=true;transcript.AcceptsReturn=true;transcript.ScrollBars=ScrollBars.Vertical;
   references.SelectedIndexChanged+=delegate{transcripts[referenceIndex]=transcript.Text;referenceIndex=references.SelectedIndex;transcript.Text=transcripts[referenceIndex];};
   Hint(L.T("Для Qwen расшифровка помогает точнее передать голос. Без неё используется копирование тембра. CosyVoice чередует записи; Qwen использует первую.","For Qwen, a transcript helps reproduce the voice more accurately. Without it, speaker embedding mode is used. CosyVoice alternates references; Qwen uses the first."));
   Hint(L.T("Для нового голоса загрузи готовый файл: 1–30 секунд, желательно 10–20 секунд чистой речи одного человека, без музыки, шума, эха и обрывов. Приложение переводит звук в формат хранения; качество и права на запись проверяет пользователь.","Import a prepared file: 1–30 seconds, ideally 10–20 seconds of clean speech from one person, without music, noise, echo or cuts. The app converts audio for storage; the user is responsible for recording quality and permissions."));
  }else{
   Label(L.T("Как показывать файл","How to display this file"));layout=new ComboBox{Width=550,DropDownStyle=ComboBoxStyle.DropDownList,DrawMode=DrawMode.OwnerDrawFixed,ItemHeight=26,BackColor=StudioColors.Surface,ForeColor=ForeColor,FlatStyle=FlatStyle.Flat};layout.Items.AddRange(new object[]{L.T("Автоматически","Automatic"),L.T("Обычная картинка / GIF","Regular image / GIF"),L.T("Спрайт Codex · 8 × 9 кадров","Codex sprite sheet · 8 × 9 cells")});layout.SelectedIndex=id.Length==0?0:Math.Max(0,Array.IndexOf(new[]{"auto","image","codex"},Convert.ToString(item["layout"])));layout.Enabled=id.Length>0;layout.DrawItem+=delegate(object sender,DrawItemEventArgs e){e.Graphics.Clear(layout.BackColor);if(e.Index>=0)TextRenderer.DrawText(e.Graphics,layout.GetItemText(layout.Items[e.Index]),Font,e.Bounds,StudioColors.Ink,TextFormatFlags.Left|TextFormatFlags.VerticalCenter);};page.Controls.Add(layout);
   Label(L.T("Предпросмотр","Preview"));var preview=new PictureBox{Width=550,Height=300,BackColor=StudioColors.Surface,SizeMode=PictureBoxSizeMode.Zoom};page.Controls.Add(preview);
   if(id.Length>0){LoadVisualPreview(preview,id);layout.SelectedIndexChanged+=delegate{LoadVisualPreview(preview,id);};}
   else{using(var pet=new PetForm())using(var bitmap=pet.RenderSnapshot()){visual=(Image)bitmap.Clone();preview.Image=visual;}}
   Hint(L.T("PNG и JPEG показываются как картинка, GIF сохраняет анимацию. WebP преобразуется в PNG. Для спрайтов поддерживается лист 1536 × 1872; в предпросмотре показан первый кадр.","PNG and JPEG display a still image; GIF keeps its animation. WebP is converted to PNG. A 1536 × 1872 sprite sheet is supported; this preview shows its first frame."));
  }
  feedback=new Label{Width=550,Height=50,ForeColor=StudioColors.Muted};page.Controls.Add(feedback);
  var actions=Row();export=Button(actions,L.T("Экспорт","Export"),async delegate{await Export();},160);export.Enabled=id.Length>0;delete=Button(actions,L.T("Удалить","Delete"),Delete,160);delete.Enabled=id.Length>0;
  var bottom=new Panel{Dock=DockStyle.Bottom,Height=70,BackColor=BackColor};Controls.Add(bottom);save=new StudioButton{Text=id.Length==0&&kind!="visual"?L.T("Сохранить копию","Save a copy"):L.T("Сохранить","Save"),Location=new Point(424,13),Width=190,Height=42};bottom.Controls.Add(save);save.Enabled=kind!="visual"||id.Length>0;save.Click+=delegate{SavePreset();};
  bottom.Controls.Add(export);bottom.Controls.Add(delete);export.SetBounds(139,13,115,42);delete.SetBounds(260,13,115,42);page.Controls.Remove(actions);actions.Dispose();
  var close=new StudioButton{Text=L.T("Закрыть","Close"),Primary=false,Location=new Point(18,13),Width=115,Height=42};bottom.Controls.Add(close);close.Click+=delegate{Close();};
  Shown+=delegate{scroll.SetContent(page);StudioColors.NativeTheme(this);};FormClosing+=delegate(object sender,FormClosingEventArgs e){if(busy){closeWhenReady=true;cancel.Cancel();e.Cancel=true;feedback.Text=L.T("Отмена…","Cancelling…");}else Stop();};
 }
 static Dictionary<string,object> ReadDefault(string path){return (Dictionary<string,object>)new JavaScriptSerializer().DeserializeObject(File.ReadAllText(path));}
 void Label(string text){page.Controls.Add(new Label{Text=text,Width=550,Height=29,Margin=new Padding(0,14,0,0)});}
 void Hint(string text){var font=new Font("Segoe UI",9);page.Controls.Add(new Label{Text=text,Width=550,Height=TextRenderer.MeasureText(text,font,new Size(550,int.MaxValue),TextFormatFlags.WordBreak).Height+18,Font=font,ForeColor=StudioColors.Muted,Margin=new Padding(0,12,0,0)});}
 TextBox Entry(string text,int limit,int height){var box=new TextBox{Multiline=height>40,Text=text,MaxLength=limit,Width=550,Height=height,BackColor=StudioColors.Surface,ForeColor=ForeColor,BorderStyle=BorderStyle.FixedSingle};page.Controls.Add(box);return box;}
 FlowLayoutPanel Row(){var row=new FlowLayoutPanel{Width=550,Height=54,Margin=new Padding(0,10,0,0)};page.Controls.Add(row);return row;}
 StudioButton Button(Control parent,string text,Action action,int width){var button=new StudioButton{Text=text,Primary=false,Width=width,Height=40,Margin=new Padding(0,0,10,0)};parent.Controls.Add(button);button.Click+=delegate{action();};return button;}
 void Stop(){if(player!=null){player.Stop();player.Dispose();player=null;}}
 void LoadVisualPreview(PictureBox preview,string id){preview.Image=null;if(visual!=null)visual.Dispose();using(var source=Image.FromFile(PersonalPresets.FilePath(root,id,"visual")))visual=(Image)source.Clone();if(layout.SelectedIndex!=1&&SpriteAtlas.Matches(visual)){var frame=new Bitmap(192,208);using(var g=Graphics.FromImage(frame))g.DrawImage(visual,new Rectangle(0,0,192,208),SpriteAtlas.Frame(0,false,false,false,false),GraphicsUnit.Pixel);visual.Dispose();visual=frame;}preview.Image=visual;}
 void Busy(bool value){busy=value;save.Enabled=!value&&(kind!="visual"||original.Length>0);export.Enabled=delete.Enabled=!value&&original.Length>0;}
 async Task PreviewVoice(){if(busy)return;Busy(true);Stop();try{string file=original.Length==0?Path.Combine(app,"assets","voice","default-en.wav"):await PersonalPresets.PreviewAudio(root,original,referenceIndex,cancel.Token,message=>feedback.Text=message);player=new SoundPlayer(file);player.Load();player.Play();feedback.Text=L.T("Воспроизводится исходная запись. Это ещё не синтез новой фразы.","Playing the reference recording. This is not a synthesized new phrase.");}catch(OperationCanceledException){feedback.Text=L.T("Отменено","Cancelled");}catch(Exception e){feedback.Text=e.Message;}finally{Busy(false);if(closeWhenReady)Close();}}
 void SavePreset(){if(busy)return;try{
  if(kind=="voice"){transcripts[referenceIndex]=transcript.Text;if(original.Length==0)ResultId=PersonalPresets.CloneDefaultVoice(root,app,name.Text.Trim(),transcripts[0]);else PersonalPresets.Update(root,original,kind,name.Text.Trim(),"",transcripts);}
  else if(kind=="character")ResultId=PersonalPresets.SaveCharacter(root,original,name.Text.Trim(),description.Text);
  else PersonalPresets.Update(root,original,kind,name.Text.Trim(),"",null,new[]{"auto","image","codex"}[layout.SelectedIndex]);
  DialogResult=DialogResult.OK;Close();
 }catch(Exception e){feedback.Text=e.Message;}}
 async Task Export(){using(var dialog=new SaveFileDialog{Filter="Kavychka preset|*.kavychka",FileName="Kavychka-"+kind+".kavychka"}){if(dialog.ShowDialog(this)!=DialogResult.OK)return;Busy(true);try{await Task.Run(delegate{PersonalPresets.Export(root,original,kind,dialog.FileName);});feedback.Text=L.T("Пресет сохранён в файл","Preset exported");}catch(Exception e){feedback.Text=e.Message;}finally{Busy(false);if(closeWhenReady)Close();}}}
 void Delete(){if(busy||original.Length==0)return;if(MessageBox.Show(this,L.T("Удалить этот пресет? Исходные загруженные файлы сохранятся. Отменить удаление нельзя.","Delete this preset? Original imported files are kept. Deletion cannot be undone."),L.T("Удалить пресет","Delete preset"),MessageBoxButtons.YesNo,MessageBoxIcon.Warning,MessageBoxDefaultButton.Button2)!=DialogResult.Yes)return;try{Stop();PersonalPresets.Delete(root,original,kind);Deleted=true;ResultId="";DialogResult=DialogResult.OK;Close();}catch(Exception e){feedback.Text=e.Message;}}
 protected override void Dispose(bool disposing){if(disposing&&!resourcesDisposed){resourcesDisposed=true;cancel.Cancel();cancel.Dispose();Stop();if(visual!=null){visual.Dispose();visual=null;}}base.Dispose(disposing);}
}

public sealed class StudioProgressForm:NightForm {
 readonly CancellationTokenSource cancel=new CancellationTokenSource();
 readonly Func<CancellationToken,Action<string>,Task<string>> operation;
 readonly Label status;
 bool finished;
 public string ResultId;
 public Exception Error;
 public StudioProgressForm(string title,Func<CancellationToken,Action<string>,Task<string>> operation){this.operation=operation;Text=title;ClientSize=new Size(540,245);StartPosition=FormStartPosition.CenterParent;BackColor=StudioColors.Background;ForeColor=StudioColors.Ink;Font=new Font("Segoe UI",10);MinimizeBox=false;MaximizeBox=false;status=new Label{Text=L.T("Подготавливаю…","Preparing…"),Location=new Point(24,62),Size=new Size(490,70)};Controls.Add(status);Controls.Add(new ProgressBar{Style=ProgressBarStyle.Marquee,Location=new Point(24,145),Size=new Size(490,7)});var stop=new StudioButton{Text=L.T("Отмена","Cancel"),Primary=false,Location=new Point(24,171),Size=new Size(180,42)};Controls.Add(stop);stop.Click+=delegate{cancel.Cancel();status.Text=L.T("Отмена…","Cancelling…");};Shown+=async delegate{try{ResultId=await this.operation(cancel.Token,message=>{if(!IsDisposed)status.Text=message;});}catch(Exception e){Error=e;}finally{finished=true;Close();}};FormClosing+=delegate(object sender,FormClosingEventArgs e){if(!finished){cancel.Cancel();e.Cancel=true;}};}
 protected override void Dispose(bool disposing){if(disposing)cancel.Dispose();base.Dispose(disposing);}
}
