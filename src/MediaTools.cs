using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Drawing;
using System.IO;
using System.IO.Compression;
using System.Linq;
using System.Net.Http;
using System.Security.Cryptography;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using System.Web.Script.Serialization;

public static class MediaTools {
 const string Url="https://files.pythonhosted.org/packages/2c/c6/fa760e12a2483469e2bf5058c5faff664acf66cadb4df2ad6205b016a73d/imageio_ffmpeg-0.6.0-py3-none-win_amd64.whl";
 const string Sha="02fa47c83703c37df6bfe4896aab339013f62bf02c5ebf2dce6da56af04ffc0a";
 const string BinarySha="2ce797a0f88d7f067180338fb227f7b1928ea727bd9a4d7a1d022f7c52af71a3";
 const long PackageBytes=31246824;
 static readonly SemaphoreSlim Gate=new SemaphoreSlim(1,1);
 public static async Task<string> Ensure(string root,CancellationToken cancel,Action<string> progress){
  await Gate.WaitAsync(cancel);try{string folder=Path.Combine(root,"media"),exe=Path.Combine(folder,"ffmpeg.exe"),marker=Path.Combine(folder,"verified.json");
   if(File.Exists(exe)){using(var hash=SHA256.Create())using(var stream=File.OpenRead(exe))if(BitConverter.ToString(hash.ComputeHash(stream)).Replace("-","").ToLowerInvariant()==BinarySha)return exe;}
   Directory.CreateDirectory(folder);string zip=Path.Combine(folder,"decoder.whl.part"),partial=exe+".part";
   progress(L.T("Загружаю декодер медиа · 31 МБ","Downloading media decoder · 31 MB"));
   using(var http=new HttpClient{Timeout=TimeSpan.FromMinutes(8)})using(var response=await http.GetAsync(Url,HttpCompletionOption.ResponseHeadersRead,cancel)){response.EnsureSuccessStatusCode();using(var input=await response.Content.ReadAsStreamAsync())using(var output=new FileStream(zip,FileMode.Create,FileAccess.Write,FileShare.None)){var buffer=new byte[81920];long done=0;int count;while((count=await input.ReadAsync(buffer,0,buffer.Length,cancel))>0){done+=count;if(done>PackageBytes)throw new IOException(AppText.T("Пакет декодера слишком велик","Media decoder package is too large"));await output.WriteAsync(buffer,0,count,cancel);progress(L.T("Загружаю декодер медиа · ","Downloading media decoder · ")+(done*100/PackageBytes)+"%");}if(done!=PackageBytes)throw new IOException(AppText.T("Декодер скачан не полностью","Media decoder download is incomplete"));}}
   using(var hash=SHA256.Create())using(var stream=File.OpenRead(zip)){if(BitConverter.ToString(hash.ComputeHash(stream)).Replace("-","").ToLowerInvariant()!=Sha)throw new IOException(AppText.T("Проверка целостности декодера не пройдена","Media decoder integrity check failed"));}
   using(var archive=ZipFile.OpenRead(zip)){var entry=archive.Entries.Single(e=>e.FullName.StartsWith("imageio_ffmpeg/binaries/",StringComparison.Ordinal)&&e.Name.EndsWith(".exe",StringComparison.Ordinal));if(entry.Length>150000000)throw new IOException(AppText.T("Неверный пакет декодера","Invalid decoder package"));entry.ExtractToFile(partial,true);
    foreach(var license in archive.Entries.Where(e=>e.Name.IndexOf("LICENSE",StringComparison.OrdinalIgnoreCase)>=0))license.ExtractToFile(Path.Combine(folder,license.Name),true);
   }
   cancel.ThrowIfCancellationRequested();if(File.Exists(exe))File.Delete(exe);File.Move(partial,exe);File.WriteAllText(marker,"{\"package_sha256\":\""+Sha+"\"}");File.Delete(zip);return exe;
  }finally{Gate.Release();}
 }
 public static string Q(string text){if(text.IndexOf('"')>=0)throw new ArgumentException(AppText.T("Неверный путь","Invalid path"));return "\""+text+"\"";}
 public static void Run(string executable,string args,CancellationToken cancel){
  cancel.ThrowIfCancellationRequested();using(var p=new Process{StartInfo=new ProcessStartInfo(executable,"-nostdin -hide_banner -loglevel error -protocol_whitelist file,pipe -y "+args){UseShellExecute=false,CreateNoWindow=true,RedirectStandardError=true}}){p.Start();var error=p.StandardError.ReadToEndAsync();
   var watch=Stopwatch.StartNew();try{while(!p.WaitForExit(100)){if(cancel.IsCancellationRequested){p.Kill();p.WaitForExit();cancel.ThrowIfCancellationRequested();}if(watch.Elapsed>TimeSpan.FromSeconds(90)){p.Kill();p.WaitForExit();throw new IOException(AppText.T("Преобразование аудио превысило время ожидания","Media conversion timed out"));}}string details=error.GetAwaiter().GetResult();if(p.ExitCode!=0)throw new IOException(L.T("Не удалось прочитать медиафайл. Проверь его формат и целостность.","Cannot read the media file. Check its format and integrity.")+"\n"+details.Substring(0,Math.Min(details.Length,700)));}finally{if(!p.HasExited){p.Kill();p.WaitForExit();}}
  }
 }
 public static void NormalizeVoice(string ffmpeg,string input,string folder,CancellationToken cancel){
  string wav=Path.Combine(folder,"preview.wav"),flac=Path.Combine(folder,"reference.flac");
  Run(ffmpeg,"-i "+Q(input)+" -t 31 -vn -ac 1 -ar 24000 -c:a pcm_s16le "+Q(wav),cancel);
  double duration=WaveSeconds(wav);if(duration<1||duration>30)throw new ArgumentException(L.T("Нужен голосовой отрывок от 1 до 30 секунд. Рекомендуется 10–20 секунд чистой речи.","Use a voice reference between 1 and 30 seconds. 10–20 seconds of clear speech is recommended."));
  Run(ffmpeg,"-i "+Q(wav)+" -c:a flac "+Q(flac),cancel);
 }
 public static double WaveSeconds(string path){using(var reader=new BinaryReader(File.OpenRead(path))){if(new string(reader.ReadChars(4))!="RIFF")throw new IOException(AppText.T("Неверный WAV-файл","Invalid WAV"));reader.ReadUInt32();if(new string(reader.ReadChars(4))!="WAVE")throw new IOException(AppText.T("Неверный WAV-файл","Invalid WAV"));int bytesPerSecond=0;long dataSize=0;bool signal=false;
   while(reader.BaseStream.Position+8<=reader.BaseStream.Length){string kind=new string(reader.ReadChars(4));uint size=reader.ReadUInt32();long end=reader.BaseStream.Position+size;if(end>reader.BaseStream.Length)throw new IOException(AppText.T("Оборванный WAV-файл","Truncated WAV"));
    if(kind=="fmt "){if(size<16)throw new IOException(AppText.T("Неверный формат WAV","Invalid WAV format"));reader.ReadUInt16();reader.ReadUInt16();reader.ReadUInt32();bytesPerSecond=reader.ReadInt32();}
    if(kind=="data"){dataSize=size;while(reader.BaseStream.Position+2<=end)if(Math.Abs((int)reader.ReadInt16())>8)signal=true;}
    reader.BaseStream.Position=end+(size%2);
   }if(bytesPerSecond<=0||!signal)throw new IOException(L.T("В файле нет слышимой речи","The audio file is silent"));return (double)dataSize/bytesPerSecond;
  }}
 public static double WaveDuration(byte[] data){using(var stream=new MemoryStream(data))using(var reader=new BinaryReader(stream)){if(stream.Length<12||new string(reader.ReadChars(4))!="RIFF")throw new IOException(AppText.T("Неверный WAV-файл","Invalid WAV"));reader.ReadUInt32();if(new string(reader.ReadChars(4))!="WAVE")throw new IOException(AppText.T("Неверный WAV-файл","Invalid WAV"));int rate=0;long samples=0;while(stream.Position+8<=stream.Length){string kind=new string(reader.ReadChars(4));uint size=reader.ReadUInt32();long end=stream.Position+size;if(end>stream.Length)throw new IOException(AppText.T("Оборванный WAV-файл","Truncated WAV"));if(kind=="fmt "&&size>=16){reader.ReadUInt16();reader.ReadUInt16();reader.ReadUInt32();rate=reader.ReadInt32();}if(kind=="data")samples=size;stream.Position=end+size%2;}if(rate<=0||samples==0)throw new IOException(AppText.T("Неверная длительность WAV","Invalid WAV duration"));return (double)samples/rate;}}
}

