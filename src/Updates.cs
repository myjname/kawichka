using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Net.Http;
using System.Reflection;
using System.Security.Cryptography;
using System.Threading;
using System.Threading.Tasks;
using System.Web.Script.Serialization;

public sealed class AppRelease {
 public Version Version;
 public string Name,Url,Sha256;
 public long Bytes;
 public static Version NormalizeVersion(string text){Version parsed;if(!Version.TryParse(text.TrimStart('v','V'),out parsed))throw new InvalidDataException(AppText.T("Неверный номер версии","Invalid release version"));return new Version(parsed.Major,parsed.Minor,Math.Max(0,parsed.Build),Math.Max(0,parsed.Revision));}
 public static AppRelease Parse(string json){
  var item=(Dictionary<string,object>)new JavaScriptSerializer().DeserializeObject(json);
  if(Convert.ToBoolean(item["draft"])||Convert.ToBoolean(item["prerelease"]))throw new InvalidDataException(AppText.T("Это не стабильный выпуск","This is not a stable release"));
  var asset=((object[])item["assets"]).Cast<Dictionary<string,object>>().Single(a=>Convert.ToString(a["name"])=="Kavychka-Setup.exe");
  string url=Convert.ToString(asset["browser_download_url"]);Uri uri;if(!Uri.TryCreate(url,UriKind.Absolute,out uri)||uri.Scheme!="https"||uri.Host!="github.com"||!uri.IsDefaultPort||!uri.AbsolutePath.StartsWith("/myjname/kawichka/releases/download/",StringComparison.Ordinal)||!uri.AbsolutePath.EndsWith("/Kavychka-Setup.exe",StringComparison.Ordinal)||!string.IsNullOrEmpty(uri.UserInfo)||!string.IsNullOrEmpty(uri.Query)||!string.IsNullOrEmpty(uri.Fragment))throw new InvalidDataException(AppText.T("Неожиданный источник обновления","Unexpected update source"));
  string digest=asset.ContainsKey("digest")?Convert.ToString(asset["digest"]):"";if(!digest.StartsWith("sha256:",StringComparison.Ordinal))throw new InvalidDataException(AppText.T("У обновления нет SHA-256","Update has no SHA-256 digest"));digest=digest.Substring(7).ToLowerInvariant();if(digest.Length!=64||digest.Any(c=>!Uri.IsHexDigit(c)))throw new InvalidDataException(AppText.T("Неверная контрольная сумма обновления","Invalid update digest"));
  long size=Convert.ToInt64(asset["size"]);if(size<1000||size>200000000)throw new InvalidDataException(AppText.T("Неверный размер установщика","Invalid installer size"));
  return new AppRelease{Version=NormalizeVersion(Convert.ToString(item["tag_name"])),Name=Convert.ToString(item["name"]),Url=url,Sha256=digest,Bytes=size};
 }
}

public static class AppUpdates {
 public const string ReleaseApi="https://api.github.com/repos/myjname/kawichka/releases/latest";
 public static Version Current {get{return AppRelease.NormalizeVersion(Assembly.GetExecutingAssembly().GetName().Version.ToString());}}
 static HttpClient Client(){var http=new HttpClient{Timeout=TimeSpan.FromMinutes(10)};http.DefaultRequestHeaders.UserAgent.ParseAdd("Kavychka/"+Current);return http;}
 public static async Task<AppRelease> Check(CancellationToken cancel){using(var http=Client()){http.Timeout=TimeSpan.FromSeconds(15);using(var response=await http.GetAsync(ReleaseApi,cancel)){if(response.StatusCode==System.Net.HttpStatusCode.NotFound)return null;response.EnsureSuccessStatusCode();var release=AppRelease.Parse(await response.Content.ReadAsStringAsync());return release.Version>Current?release:null;}}}
 public static async Task<string> Download(AppRelease release,string root,CancellationToken cancel,Action<long,long> progress){
  string folder=Path.Combine(root,"updates",release.Version.ToString());Directory.CreateDirectory(folder);string target=Path.Combine(folder,"Kavychka-Setup.exe"),partial=target+".part";
  if(File.Exists(target)&&Verify(target,release))return target;
  using(var http=Client())using(var response=await http.GetAsync(release.Url,HttpCompletionOption.ResponseHeadersRead,cancel)){response.EnsureSuccessStatusCode();using(var input=await response.Content.ReadAsStreamAsync())using(var output=new FileStream(partial,FileMode.Create,FileAccess.Write,FileShare.None)){var buffer=new byte[81920];long done=0;int count;while((count=await input.ReadAsync(buffer,0,buffer.Length,cancel))>0){done+=count;if(done>release.Bytes)throw new InvalidDataException(AppText.T("Размер обновления превышает ожидаемый","Update exceeds expected size"));await output.WriteAsync(buffer,0,count,cancel);progress(done,release.Bytes);}}}
  cancel.ThrowIfCancellationRequested();if(!Verify(partial,release)){File.Delete(partial);throw new InvalidDataException(L.T("Проверка обновления не пройдена. Рабочая версия сохранена.","Update verification failed. The installed version is unchanged."));}
  if(File.Exists(target))File.Delete(target);File.Move(partial,target);return target;
 }
 public static bool Verify(string file,AppRelease release){if(new FileInfo(file).Length!=release.Bytes)return false;using(var hash=SHA256.Create())using(var stream=File.OpenRead(file))return BitConverter.ToString(hash.ComputeHash(stream)).Replace("-","").ToLowerInvariant()==release.Sha256;}
}
