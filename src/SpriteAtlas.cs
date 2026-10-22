using System.Drawing;

public static class SpriteAtlas {
 // Public Codex atlas contract: 8 x 9 cells, each 192 x 208 pixels.
 // https://github.com/openai/skills/blob/main/skills/.curated/hatch-pet/references/codex-pet-contract.md
 public static bool Matches(Image image){return image.Width==1536&&image.Height==1872;}
 public static Rectangle Frame(long elapsedMs,bool paused,bool speaking,bool teasing,bool happy){
  int row=happy?4:speaking?3:teasing?8:0,column=0;
  if(paused)row=0;
  else if(row==0){int[] durations={1680,660,660,840,840,1920};long time=elapsedMs%6600;for(int i=0;i<durations.Length;i++){if(time<durations[i]){column=i;break;}time-=durations[i];}}
  else column=(int)((elapsedMs/140)%(row==4?5:row==3?4:6));
  return new Rectangle(column*192,row*208,192,208);
 }
}
