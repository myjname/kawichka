using System;
using System.IO;
using System.Text;

public static class SettingsFile {
 public static void Write(string path,string json){
  Atomic(path,temp=>File.WriteAllText(temp,json,Encoding.UTF8));
 }
 public static void WriteBytes(string path,byte[] data){Atomic(path,temp=>File.WriteAllBytes(temp,data));}
 static void Atomic(string path,Action<string> write){
  string temp=path+".tmp-"+Guid.NewGuid().ToString("N");
  try{write(temp);if(File.Exists(path))File.Replace(temp,path,null);else File.Move(temp,path);}
  finally{try{if(File.Exists(temp))File.Delete(temp);}catch(IOException){}catch(UnauthorizedAccessException){}}
 }
}
