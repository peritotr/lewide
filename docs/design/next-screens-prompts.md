# Sonraki ekranlar için tasarım promptları

Beş prompt var, uygulamada bana lazım olma sırasına göre dizili. Her birini ayrı ayrı, aynı Claude Design oturumuna (sorgu editörünü tasarlayan oturuma) yapıştırın; hepsi ortak girişe dayanıyor, o yüzden **ortak girişi her promptun başına ekleyin**.

Sıra: 1 ve 2 şu an uygulamayı bekletiyor, 3 ve 4 bir sonraki geliştirme adımı, 5 hepsini toparlıyor.

---

## Ortak giriş (her promptun başına)

Sorgu editörü ekranında **1a "Drafting table"** yönünü seçtik ve uyguladık. Bundan sonraki her şey 1a ile ve mevcut `tokens.css` ile tutarlı olmalı: kare köşeler, 1px çizgiler, tek çelik mavisi vurgu, Barlow / Barlow Condensed / IBM Plex Mono, kayıt işaretleri yalnızca birincil düğmede ve panel kesişimlerinde. 1b'yi bırakıyoruz.

Uygulamadan gelen kararlar, tasarımda bunlara uy:

- **Marka logosu kullanma.** Hukuki kontrol bitene kadar her veritabanı türü iki harfli rozetle gösteriliyor (`Pg`, `My`, `Sq`, `Ms`, `Or`, `Mg`, `Rd` ...). Rozet varsayılan görünüm olsun.
- **Sayılar tabloda ham gösteriliyor** (`2184.50`, binlik ayırıcı yok), çünkü görünen değer kopyalanan değerle aynı olmalı.
- **Sorgu sonucunun neden salt okunur olduğunu uygulama bilemiyor.** Metin "Read-only · query result"; "aggregate result" yazma.
- **Ağaçtaki satır sayıları yaklaşık değerdir** ve geç gelebilir; sayı yokken satırın nasıl göründüğünü de göster.
- Arayüz metinleri İngilizce, çeviri payı bırakılmış.

Teslim biçimi öncekiyle aynı olsun: 1440×900, açık ve koyu tema, gerçekçi örnek veri (`acme-prod` / `shop.public` senaryosunu sürdür), İngilizce README içinde kesin ölçüler ve davranış notları. `tokens.css` dosyasındaki mevcut değerleri değiştirme; yeni değişken gerekiyorsa ekle ve README'de eklediklerini listele. Tasarlamadığın durumları README'de açıkça "tasarlanmadı" diye işaretle.

---

## Prompt 1 — Bağlantı penceresi ve bağlantı yönetimi

Yeni bağlantı oluşturma ve var olan bağlantıları yönetme akışını tasarla. Şu an bu pencere tasarımsız, düz bir formla çalışıyor.

Tasarlanacaklar:

1. **Veritabanı türü seçimi.** Veri modeline göre gruplanmış (Relational, Distributed SQL, Analytic, Document, Key-value, Vector, Graph, Time series, Search), aranabilir ızgara. Her kart: iki harfli rozet, ad, model etiketi. Henüz desteklenmeyen türler için "yakında" görünümü.
2. **Bağlantı formu**, sekmeli: General, SSH tunnel, TLS, Advanced.
   - General: ad, ortam (Local / Staging / Production), renk (dört bağlantı rengi), sunucu, port, kullanıcı, parola, veritabanı. Dosya tabanlı türlerde (SQLite) bunların yerine dosya yolu ve "Browse".
   - Parola alanının yanında "Save in system keychain" seçeneği.
   - SSH tunnel: açma/kapama, sunucu, port, kullanıcı, kimlik doğrulama (parola veya özel anahtar dosyası + anahtar parolası).
   - TLS: üç mod (Off / Verify certificate / Encrypt only), CA sertifika dosyası. "Encrypt only" seçilince güvenlik uyarısı.
   - Advanced: sürücüye özel anahtar-değer seçenekleri.
3. **Test connection** düğmesi ve dört sonucu: test ediliyor, başarılı (sunucu sürümü ve gecikme), hatalı (sunucudan gelen hata metni, seçilebilir), zaman aşımı.
4. **SSH sunucu anahtarı onayı.** Sunucu `known_hosts` içinde yokken parmak izini (`SHA256:...`) gösterip onay isteyen görünüm. Ayrıca **anahtar değişmiş** durumu: bu bir güvenlik uyarısıdır, onay düğmesi sunulmaz, bağlantı reddedilir.
5. **Production onayı.** Ortam Production seçildiğinde formun ve kaydetme düğmesinin nasıl değiştiği.
6. **Düzenleme modu.** Kayıtlı bağlantıyı düzenlerken parola alanı: kayıtlı parola gösterilmez, "değiştirilmedi" durumu.
7. **Parola sorma penceresi.** Parolası kaydedilmemiş bir bağlantıya tıklanınca açılan küçük pencere.
8. **Kenar çubuğundaki bağlantı satırının bağlam menüsü:** Connect / Disconnect, Edit, Duplicate, New query, Remove. Remove için onay görünümü.
9. **Bağlantı satırının ek durumları:** bağlanıyor, bağlantı hatası (hata metnine nasıl ulaşılır), bağlantı koptu.
10. **İlk açılış.** Hiç bağlantı yokken ana pencerenin boş durumu ve "New connection" çağrısı.

---

## Prompt 2 — Sorgu editörünün eksik durumları

Sorgu editörü ekranında "tasarlanmadı" diye bıraktığın durumları tasarla. Ekranın yerleşimi değişmiyor; her durumu ilgili bölgeyi göstererek ver.

1. **Çalışan sorgu.** Run / Stop düğmelerinin hali, sonuç panelinde ilerleme göstergesi ve geçen süre, durum çubuğu. 2 saniyeyi geçen uzun sorgu için görünüm. İptal edildikten sonraki hal.
2. **Sorgu hatası.** Sonuç panelinde hata (hata kodu, mesaj, varsa ayrıntı ve ipucu; metin seçilebilir), editörde hatalı satırın ve konumun işaretlenmesi, sekmede hata işareti.
3. **Messages görünümü.** Birden fazla ifade çalıştığında her ifade için bir satır: ifade önizlemesi, sonuç ("3 rows affected" / "100 rows returned"), süre, hata varsa hata. Sunucu bildirimleri (NOTICE, WARNING) için görünüm.
4. **Query plan görünümü.** `EXPLAIN` çıktısı: düğüm ağacı, her düğümde tahmini maliyet ve satır sayısı, en pahalı düğümün vurgulanması. Ham metin görünümüne geçiş.
5. **Boş ve kısmi sonuçlar.** Satır dönmeyen sorgu; satır döndürmeyen ifade (yalnızca etkilenen satır sayısı); satır sınırına takılan sonuç ("first 1,000 rows shown" ve sınırı yükseltme çağrısı).
6. **Otomatik tamamlama açılır listesi.** Tablo, kolon, anahtar sözcük ve fonksiyon önerileri; her öğede tür simgesi ve sağda ayrıntı (kolon tipi, tablonun şeması). Seçili öğe, klavye ipuçları.
7. **Araç çubuğu menüleri.** Bağlam seçici (bağlantı / veritabanı / şema), Limit, işlem modu (Auto-commit / Manual). Manual modda açık işlem varken araç çubuğunda Commit ve Rollback, durum çubuğunda uyarı.
8. **Production'da yıkıcı ifade onayı.** `DELETE`, `DROP`, `TRUNCATE` veya `WHERE` içermeyen `UPDATE` için onay penceresi: ifadenin kendisi, bağlantı adı, onay için bağlantı adını yazma alanı.
9. **Sekme durumları ve menüsü.** Kaydedilmemiş nokta, çalışıyor, hata; sekme çok olduğunda taşma; bağlam menüsü (Close, Close others, Duplicate, Rename).
10. **Tablo hücresi.** Bağlam menüsü (Copy, Copy as JSON, Copy row, Filter by value); uzun metin ve JSON değerler için açılır hücre görüntüleyici; ikili veri, boolean ve `NULL` gösterimleri; kolon başlığı menüsü.
11. **Bağlantısız sekme.** Sekmenin bağlantısı kapalıyken veya hiç yokken editörün ve araç çubuğunun hali.
12. **Kaydetme.** Save ile sorguyu adlandırma ve kayıtlı sorguların nerede listelendiği.

---

## Prompt 3 — Tablo verisi görünümü

Kenar çubuğunda bir tabloya çift tıklayınca açılan veri görünümünü tasarla. Sorgu sonucu tablosunun görsel dilini kullan; fark, bu tablonun **düzenlenebilir** olması.

1. **Yerleşim.** Sekme türü "table data". Üstte filtre ve sıralama çubuğu, ortada tablo, altta sayfalama ve satır sayısı. Görünüm değiştirici: Data / Structure.
2. **Filtre çubuğu.** Kolon + operatör + değer koşulları, birden fazla koşul, ham `WHERE` yazma seçeneği, etkin filtre sayısı, temizleme.
3. **Kolon başlığı.** Ad, tip, birincil anahtar ve yabancı anahtar işaretleri, sıralama, genişletme tutamağı. Yabancı anahtar hücresinden ilgili satıra gitme.
4. **Hücre düzenleme.** Düzenleme halindeki hücre; tipe göre editörler (metin, sayı, boolean, tarih-saat, enum listesi, JSON, uzun metin); hücreyi `NULL` yapma; geçersiz değer hatası.
5. **Bekleyen değişiklikler.** Değişmiş hücre, eklenen satır ve silinmek üzere işaretlenen satır için üç ayrı işaret; anlam yalnızca renkle taşınmasın. Altta "3 changes · Preview · Apply · Discard" çubuğu.
6. **SQL önizleme penceresi.** Uygulanacak `INSERT` / `UPDATE` / `DELETE` ifadeleri, tek işlemde uygulanacağı bilgisi, Production'da ek onay.
7. **Uygulama sonuçları.** Başarılı; kısmen başarısız (hangi satır, hangi hata; değişiklikler kaybolmaz).
8. **Salt okunur durum.** Birincil anahtarı olmayan tabloda ve görünümlerde (view) düzenlemenin kapalı olduğunu açıklayan bilgi şeridi.
9. **Büyük tablo.** Sayfalama denetimleri, sayfa boyutu, toplam satır sayısı yaklaşıkken ve sayılırken görünüm.
10. **Satır seçimi.** Çoklu satır seçimi, seçim üzerinde toplu işlemler (Copy, Delete, Export).
11. **Durumlar.** Yükleniyor, boş tablo, filtreye uyan satır yok, yükleme hatası.

---

## Prompt 4 — Tablo yapısı görünümü

Bir tablonun yapısını gösteren ve değiştiren görünümü tasarla (sekme türü "table structure", sekme başlığı `orders · Structure`).

1. **Alt sekmeler:** Columns, Indexes, Foreign keys, Triggers, DDL.
2. **Columns.** Sıra, ad, tip, nullable, varsayılan değer, birincil anahtar, açıklama. Salt okunur liste hali ile düzenleme hali; kolon ekleme, silme, sırayı değiştirme.
3. **Indexes.** Ad, kolonlar, unique, yöntem (btree vb.), boyut.
4. **Foreign keys.** Ad, kolonlar, hedef tablo ve kolonlar, `ON DELETE` / `ON UPDATE` davranışı; hedef tabloya gitme.
5. **Triggers.** Ad, olay, zamanlama, fonksiyon.
6. **DDL.** Tablonun `CREATE TABLE` ifadesi, sözdizimi renkli, salt okunur, kopyalama düğmesi.
7. **Değişiklikleri uygulama.** Bekleyen yapı değişikliklerinin işaretlenmesi, üretilecek `ALTER TABLE` ifadelerinin önizlemesi, Production'da ek onay, tabloyu kilitleyebilecek değişiklikler için uyarı.
8. **Görünümler (view) için hal:** yalnızca kolonlar ve tanım sorgusu.
9. **Durumlar.** Yükleniyor, yetki yok, sürücünün desteklemediği alt sekmenin nasıl göründüğü (ör. SQLite'ta olmayan özellikler).

---

## Prompt 5 — Bileşen kütüphanesi, komut paleti ve ayarlar

Buraya kadar tasarlanan ekranlardaki parçaları tek bir bileşen kütüphanesinde topla ve kalan iki genel ekranı tasarla.

1. **Bileşen kütüphanesi sayfası.** Her bileşen tüm durumlarıyla (varsayılan, hover, odak, aktif, devre dışı, hata), açık ve koyu temada, Compact ve Comfortable yoğunlukta:
   düğme (birincil, ikincil, hayalet, tehlikeli), simge düğmesi, metin girişi, parola girişi, seçim kutusu, açılır menü, bağlam menüsü, onay kutusu, radyo, anahtar (switch), bölümlü seçici, sekme (pencere sekmesi, alt sekme), ağaç satırı, tablo başlığı ve hücresi, etiket (ortam etiketi dahil), rozet (veritabanı türü), ipucu (tooltip), bildirim (toast), bilgi şeridi (bilgi, uyarı, hata), modal pencere, onay penceresi, ilerleme göstergesi (belirli ve belirsiz), boş durum bloğu, ayırıcı ve boyutlandırma tutamağı, kaydırma çubuğu.
2. **Koyu tema gölgeleri.** `tokens.css` içindeki koyu tema `--shadow-*` değerlerini açılır menü, modal ve tooltip üzerinde görsel olarak doğrula; gerekiyorsa düzelt.
3. **Komut paleti** (`⌘K` / `Ctrl+K`). Arama alanı; gruplanmış sonuçlar (Tables, Connections, Commands, Saved queries, Recent); her sonuçta tür simgesi, bağlantı işareti ve kısayol; seçili öğe; sonuç yok durumu; `>` ile yalnızca komut arama.
4. **Ayarlar.** Bölümler: Appearance (tema: System / Light / Dark, yoğunluk, arayüz ve editör yazı tipi boyutu), Editor (sekme genişliği, satır kaydırma, otomatik tamamlama, anahtar sözcükleri büyük harfe çevirme), Data (varsayılan satır sınırı, sayı ve tarih gösterimi, `NULL` gösterimi), Keyboard shortcuts (aranabilir liste, kısayol değiştirme, çakışma uyarısı), Security (Production onayları, parolaların anahtar zincirinde saklanması, SSH `known_hosts` konumu), About.
5. **Dar pencere.** Ana pencerenin 1024×700 hali ve kenar çubuğunun daraltılmış hali.
6. **Windows / Linux başlık çubuğu** varyantının sorgu editöründe tam görünümü.

Bu prompttan sonra sıradakiler: içe/dışa aktarma sihirbazı ve ilişkisel olmayan görünümler (doküman, anahtar-değer, vektör, graf). Onları ilgili sürücüler yazılmaya başlandığında isteyeceğim.

---

## Prompt 6 — Yapay zekâ yardımcısı

Uygulamaya yapay zekâ yardımcısı ekliyoruz. Kullanıcı kendi API anahtarını tanımlar; yardımcı sorgu yazmaya, var olan sorguyu incelemeye ve iyileştirmeye yardım eder. Ortak girişteki kurallar geçerli; ayrıca şu ilkelere uy:

- **Yardımcı hiçbir şeyi kendiliğinden çalıştırmaz.** Önerdiği SQL editöre eklenir veya editördeki metnin yerine geçer; çalıştırmak her zaman kullanıcının işidir ve Production onayları aynen geçerlidir.
- **Neyin gönderildiği her zaman görünür.** Varsayılan olarak yalnızca şema (tablo, kolon, tip, indeks adları) ve sorgu metni gider; satır verisi gitmez. Kullanıcı bunu görebilmeli ve değiştirebilmeli.
- Yapay zekâya ait öğeler için ayrı bir renk ekleme; mevcut vurgu rengini ve tek bir simgeyi (Lucide `sparkles`) kullan.

Tasarlanacaklar:

1. **Ayarlar › AI.** Sağlayıcı listesi: Anthropic, OpenAI, Google Gemini ve "OpenAI-compatible endpoint" (Ollama, OpenRouter, Groq, Mistral gibi servisler için; ad, adres, anahtar). Her sağlayıcı için: API anahtarı alanı (kaydedilen anahtar gösterilmez, "Saved in system keychain" durumu), "Test" düğmesi ve sonuçları, model seçimi, kaldırma. Varsayılan sağlayıcı ve model seçimi. Hiç sağlayıcı yokken boş durum.
2. **Ayarlar › AI › Gizlilik.** "Şema gönder" (açık), "Örnek satır gönder" (kapalı), "Sorgu sonuçlarını gönder" (kapalı) anahtarları; her birinin altında tam olarak neyin gittiğini anlatan kısa metin. Production bağlantıları için ayrı, daha sıkı varsayılanlar. Yerel modelde (Ollama) verinin cihazdan çıkmadığı bilgisi.
3. **Sorgu editöründe Ask AI.** Araç çubuğunda düğme ve kısayol (`⌘I`). Açılınca editörün yanında veya altında bir panel: hazır eylemler (Explain this query, Check for mistakes, Optimize, Add comments) ve serbest soru alanı. Hangi metin üzerinde çalıştığı belli olsun: seçim varsa seçim, yoksa imleçteki ifade.
4. **İnceleme sonucu.** Sorgunun doğruluğu: sorun yok / uyarılar / hatalar. Her bulgu: önem derecesi, açıklama, editördeki satıra bağlantı. Editörde ilgili satırın işaretlenmesi.
5. **Öneri ve tek tuşla uygulama.** Önerilen sorgu, mevcut sorguyla **fark (diff) görünümünde**: eklenen ve silinen satırlar, anlam yalnızca renkle taşınmasın. Düğmeler: Apply (editördeki metni değiştirir, geri alınabilir), Insert below, Copy, Dismiss. Uygulandıktan sonraki durum ve "Undo".
6. **AI sohbet sekmesi.** Bağlantıya bağlı yeni bir sekme türü ("AI chat", sekme simgesi `sparkles`). Solda konuşma, her yanıttaki SQL bloğunda Insert into editor / Open in new query / Copy düğmeleri. Üstte bağlam çubuğu: hangi bağlantı ve şema, kaç tablo bağlama dahil, "nelerin gönderildiğini gör" bağlantısı. Yardımcının açıklayıcı soru sorduğu durum (ör. "Hangi tarih kolonunu kastediyorsunuz?") ve hazır yanıt düğmeleri.
7. **Tavsiyeler.** Yardımcının sorguyla ilgili yan önerileri: eksik indeks (önerilen `CREATE INDEX` ifadesi, tahmini etki, Production'da uyarı), tam tablo taraması, eksik `LIMIT`, `SELECT *`. Bir tavsiye kartının görünümü ve ifadeyi yeni sorgu sekmesinde açma.
8. **Durumlar.** Yanıt yazılırken (akış halinde metin, Stop düğmesi); hata (geçersiz anahtar, kota doldu, ağ yok, model bulunamadı); sağlayıcı tanımlı değilken Ask AI'a basılınca ayarlara yönlendiren görünüm; çok büyük şemada bağlamın kırpıldığı uyarısı.
9. **Gönderilenleri gör penceresi.** Bir isteğin tam içeriği: sistem talimatı, şema özeti, sorgu metni; salt okunur, kopyalanabilir.
10. **Kullanım.** Yanıtın altında küçük bilgi satırı: model adı, giriş ve çıkış token sayısı.
