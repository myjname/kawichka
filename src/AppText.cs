using System;
using System.Collections.Generic;
using System.Globalization;

// Shared by the desktop app, setup executable and standalone storage checks.
public static class AppText {
 public static string Language=CultureInfo.CurrentUICulture.TwoLetterISOLanguageName=="ru"?"ru":"en";
 static readonly object gate=new object();
 static readonly Dictionary<string,string[]> captions=new Dictionary<string,string[]>();
 public static string T(string ru,string en){lock(gate){var pair=new[]{ru,en};captions[ru]=pair;captions[en]=pair;}return Language=="ru"?ru:en;}
 public static string Caption(string value){lock(gate){string[] pair;return value!=null&&captions.TryGetValue(value,out pair)?pair[Language=="ru"?0:1]:value;}}
 public static string Parts(string value){if(value==null)return value;lock(gate){var pairs=new List<string[]>(captions.Values);pairs.Sort((a,b)=>b[Language=="ru"?1:0].Length.CompareTo(a[Language=="ru"?1:0].Length));foreach(var pair in pairs){string from=pair[Language=="ru"?1:0],to=pair[Language=="ru"?0:1];if(from.Length>0&&from!=to)value=value.Replace(from,to);}return value;}}
}
