-- Synthetic fixture. No user recordings, model weights, or credentials.
CREATE TABLE settings(key TEXT PRIMARY KEY,value TEXT NOT NULL);
CREATE TABLE phrases(
 id TEXT PRIMARY KEY,text TEXT NOT NULL,normalized TEXT UNIQUE NOT NULL,
 category TEXT NOT NULL,model TEXT NOT NULL,profile TEXT NOT NULL,
 cache_key TEXT,audio_path TEXT,approved INTEGER NOT NULL DEFAULT 0,
 day TEXT,used_at TEXT,created_at TEXT NOT NULL,deleted INTEGER NOT NULL DEFAULT 0,
 seed INTEGER NOT NULL DEFAULT 42,voice_style TEXT NOT NULL DEFAULT '',
 voice_instruction TEXT NOT NULL DEFAULT '',voice_reference TEXT NOT NULL DEFAULT '',
 followup INTEGER NOT NULL DEFAULT 0,language TEXT NOT NULL DEFAULT 'ru',
 rejected INTEGER NOT NULL DEFAULT 0);
CREATE TABLE plans(day TEXT PRIMARY KEY,target INTEGER NOT NULL,status TEXT NOT NULL);
CREATE TABLE voice_rotation(id INTEGER PRIMARY KEY,next_index INTEGER NOT NULL);
CREATE TABLE preparation(slot TEXT PRIMARY KEY,day TEXT NOT NULL,category TEXT NOT NULL,
 followup INTEGER NOT NULL DEFAULT 0,phrase_id TEXT,state TEXT NOT NULL DEFAULT 'pending',
 attempts INTEGER NOT NULL DEFAULT 0,retry_at REAL NOT NULL DEFAULT 0,error TEXT NOT NULL DEFAULT '');
INSERT INTO phrases(id,text,normalized,category,model,profile,cache_key,audio_path,
 approved,day,used_at,created_at,deleted,voice_style,voice_instruction,voice_reference,language)
 VALUES('used','Отдохни несколько минут.','отдохни несколько минут','break','tts-cosyvoice3',
 'voice1','old-cache-key','ready.flac',1,'2026-10-23','2026-10-23T10:00:00Z',
 '2026-10-23T09:00:00Z',0,'confident','Уверенно','one.wav','ru');
INSERT INTO phrases(id,text,normalized,category,model,profile,created_at,deleted,rejected)
 VALUES('excluded','Другая фраза.','другая фраза','lunch','tts-0.6b','voice1','2026-10-22',1,1);
INSERT INTO plans VALUES('2026-10-23',13,'ready');
INSERT INTO voice_rotation VALUES(1,5);
INSERT INTO preparation(slot,day,category,phrase_id,state,attempts,retry_at)
 VALUES('slot-1','2026-10-23','break','used','retry',2,1792756800);
