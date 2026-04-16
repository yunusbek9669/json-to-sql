# Universal Adaptive Query (UAQ) Engine

UAQ (Universal Adaptive Query) — bu Tizimning turli xil komponentlari hamda backend-frontend o'rtasidagi ma'lumot almashinuvini inqilobiy darajada osonlashtirish uchun yaratilgan dinamik JSON-to-SQL kompilyatori. Rust tilida yozilganligi tufayli eng yuqori darajadagi tezlikda (Native) ishlaydi va har qanday til (PHP, Java, Node.js, Python, va hk) bilan xavfsiz (FFI texnologiyasi asosida) integratsiya qilinadi.

Ushbu qollanma UAQ dvigateli haqida to‘liq tasavvurga ega bo'lish, hamda **Backend** va **Frontend** dasturchilari o'rtasidagi kelishuvni mukammal qilish uchun qo'llanma hisoblanadi. 

---

## 🛠 Backend Dasturchilar Uchun Qo'llanma

Backend dasturchining asosiy vazifasi — qat'iy Xavfsizlik (Whitelist) qoidalarini belgilash hamda bazadagi jadvallar o'zaro qanday aloqada ekanligini bildirishdan iborat. Shundan so'ng qolgan barcha og'ir yukni (SQL kod yozish va ma'lumotni tuzish ishlari) UAQ Engine to'la o'z zimmasiga oladi.

### 1. Tizim Integratsiyasi (C-FFI)
Kutubxona C tili arxitekturasi yordamida barcha platformalar bilan ulanadi. C-Header tarifi:
```c
char* uaq_parse(
    const char* json_input,     // Frontenddan kelgan JSON so'rov
    const char* whitelist_json, // Backendda yozilgan Whitelist (Xavfsizlik)
    const char* relations_json, // Jadvallar aloqalari (Foreign keys)
    const char* macros_json     // Ixtiyoriy: Oldindan yozilgan murakkab makroslar
);

void uaq_free_string(char* s);  // Xotirani tozalash uchun
```
> *Backend shunchaki Middleware sifatidadigini xizmat qiladi: frontend so'rovini kutubxonaga beradi, qaytgan SQL natini xavfsiz ishlatib (`:p1`, `:p2` kabi parametrlar bilan limitlagan holda) ma'lumotlarni chiqarib beradi.*

### 2. Whitelist: Xavfsizlik Qatlami
Bunda frontend bazadagi qaysi jadvalni qanday taxallus (alias) ostida ko'rishi va qaysi ustunlarga kira olishi belgilab qo'yiladi.

Format: `"haqiqiy_jadval : taxallus_nomi"`

*   **Ruxsat berish (Massiv):** `["*"]` yozilsa, barcha ustunlarga ochiq ruxsat etiladi (avtomat tekshiriladi).
*   **Virtual yoki Obfuskatsiya Qilingan Ustunlar (Obyekt):** Obyekt ko'rinishida kalit uzatish imkonini beradi. Shunda frontend faqat qat'iy cheklangan virtual-qiymatlarga etib boradi.
```json
{
  "user_profile:profile": ["*"],
  "employee:emp": {
    "id": "id",
    "full_name": "CONCAT(last_name, ' ', first_name)" // Frontend faqat full_name ko'ra oladi
  }
}
```

### 3. Relations: Jadvallarni Bog'lash (Auto-Join)
Backend faqatgina ikki jadvallar o'rtasidagi yo'nalishni bir marta konfiguratsiyaga kiritib qo'yadi. UAQ graf orqali eng murakkab bo'lgan yo'llarni ham (BFS yordamida) o'zi topib avtomatlashtirilgan JOIN larni qura oladi. Shuningdek JOIN turiga qattiq bog'lanib qolmaslik uchun qoida `LEFT JOIN` deb emas, `@join` shaklida yozilishi maqsadga muvofiq:
```json
{
  "emp->profile": "@join @table ON @1.id = @2.user_id AND @2.status = 1"
}
```
*   **Standart JOIN turlari o'qilishi:** Kod kalitdagi belgiga asoslanib asosiy ulanish qanday ekanini anglaydi: 
    *   `->` = `LEFT JOIN`
    *   `<-` = `RIGHT JOIN`
    *   `-><-` = `INNER JOIN`
    *   `<->` = `FULL JOIN`
Biroq bu ko'rinish faqatgina *standart* holat hisoblanadi. Agar frontend dasturchi boshqacha ulanishga ehtiyoj sezsa, bu qarorni bemalol o'z qo'liga ola biladi (pastdagi `$join` bo'limiga qarang)!


---

## 💻 Frontend Dasturchilar Uchun Qo'llanma

Odatda har bir yangi filtr yoki qoshimcha bog'langan jadval uchun backendda alohida API (Endpoint) yozilishi kerak bo'ladi. Ushbu tizimning eng asosiy taklifi esa: **Yagona Endpoint** tushunchasidir!
Endi har safar yangi ehtiyoj tug'ilganda backend dasturchilarni bezovta qilmaysiz. Yagona API orqali, qanday murakkab ierarxiya (Daraxt/Piramida) shaklida o'zingizga ma'lumot kerak bo'lsa, xuddi shu strukturani tushuntiruvchi shunchaki bitta JSON yuborasiz (Graph kabi)! 

Tizim PostgreSQL ning eng ilg'or funksiyalari (LATERAL, JSON_BUILD_OBJECT, va JSON_AGG) orqali barcha chigal munosabatlarni qisqa vaqt ichida tayyor javob ko'rinishida beradi. 

### 1. Asosiy Kalit So'zlar (Direktivalar)

| Direktiva  | Vazifasi                                                                   | Majburiy |
|------------|----------------------------------------------------------------------------|----------|
| `@data`    | Yagona obyekt bazadan olish uchun (Root node sifatida qat'iy).             | Ha       |
| `@data[]`  | Ma'lumotlarni massiv (ro'yxat qilib `[{}, {}]`) olish uchun.               | Ha       |
| `@source`  | Manba aliasi (Backenddagi whitelist ruxsati asosida) va filtrlar.          | Ha       |
| `@fields`  | Natijada sizga aniq qaysi maydon kalitlari qaytib kelishi zarurligi.       | Yo'q     |
| `@flatten` | Joriy tugun ichkisini bitta ustki (`ota`) qavatga yoyib (merge) yuborish.  | Yo'q     |
| `[]`       | İstagan rolingiz (tugun nomi) oxiriga ulasangiz, ichki ARRAY olib kelasiz. | Yo'q     |

### 2. Soddadan Murakkabga So'rov Yozish

**Birinchi Qadam: Oddiy Ro'yxat o'qish (Paginatsiya bilan)**  
Eng sodda holatda jadval ma'lumlarini tartib bilan, 10 tagacha cheklov bilan olaylik.

```json
{
  "@data[]": {
    "@source": "emp[status: 1, age: >25, $limit: 10, $order: id DESC]",
    "@fields": ["id", "full_name"]
  }
}
```
*Iltimos diqqat qiling:* Filtrlar (`:`, `!:`, `>`, `<`, `..`, `~`, `in`) to'g'ridan-to'g'ri `@source` da konfiguratsiya qilinadi. Cheklovlar (`$limit`, `$offset`, `$order`) sharti qo'llaniladi.

**Jadvalararo JOIN turini dinamik o'zgartirish (`$join`)**  
Frontend endi ma'lum bir jadval ro'yxatini ichidan ulanishni qo'lda boshqarish imkoniyatiga ega. Qoida tariqasida kelayotgan o'zgarishlar (INNER yoki LEFT bo'lishi backendda belgilanadi), biroq agar siz aniq mos kelmagan ma'lumotlarni ham (LEFT JOIN kabi) olmoqchi bo'lsangiz, o'zingiz tanlash huquqiga egasiz! Buni `@source` ichida bajarilib:
`"@source": "positionCteTable[is_current: true, $join: ->]"` yoki to'liq so'z orqali `"$join: LEFT"` deb berSangiz kifoya qilingan. Boshqa operatorlar (`<-`, `-><-`) ham tegishli ulanish turini o'zgartiradi. Endi ustunlik sizning qo'lingizda!

**Ikkinchi Qadam: Obyekt ichida qo'shimcha ifodalar (`@fields` obyekti) bilan**  
Agar faqat o'zingiz yozgan qaytish formatni (Masalan frontda sanani vizualizatsiya qilish kabi ifodalarni) qat'iy o'zlashtirish uchun:

```json
{
  "@data": {
    "@source": "profile",
    "@fields": {
      "front_id": "id",
      "sana": "TO_CHAR(TO_TIMESTAMP(created_time), 'DD.MM.YYYY')" 
    }
  }
}
```

**Uchinchi Qadam: Ichma-ich Array (One-to-Many qilib ko'tarish) va Flatten**
Agar xodimga va uning qo'shimcha ro'yxat shaklidagi `positions` hamda `educations` lariga ehtiyoj bo'lsa:
```json
{
  "@data": {
    "@source": "emp[status: 1]",
    "@fields": { "id": "id", "full_name": "full_name" },
    
    "positions[]": {
      "@source": "department_staff_position[is_current: true]",
      "@fields": {
        "id": "id",
        "begin_date": "TO_CHAR(TO_TIMESTAMP(start_time), 'DD.MM.YYYY')"
      },
      "ishJoyi": {
        "@source": "org[status: 1]",
        "@flatten": true, 
        "@fields": {
          "viloyat_boshqarmasi": "name"
        }
      }
    },
    
    "educations[]": {
       "@source": "education[$limit: 10, $order: id DESC]",
       "@fields": {
          "id": "unique",
          "diploma_type": "diploma_name"
       }
    }
  }
}
```



---

## 🏆 E'tibor beriladigan afzalliklar (Why UAQ?)

- **Sekiroz Data Parsing**: Tizim to'gridan to'gri PostgreSQL kuchi bilan Serverga bog'langan eng optimal JSON generator strukturasida javob jo'natadi, buning uchun ORM/SQL Query builderlarni "kuzatib yotish" necha o'nlab MB backend server operativ xotirasini tejaydi.
- **Auto-Join Tizimi**: O'rtada yuzlab ulanadigan qidiruv jadvali bo'lsa ham (A -> C), siz faqat so'rang, UAQ avtomat eng qisqa yo'lni qidirib (A -> B -> C) graflar mantiqida ulashadi.
- **Nol (Zero) SQL Kiritish Xavfi (No SQL Injection)**: Yuborayotgan barcha filtrlar qat'iy tahlil qilinib `PDO / Prepared Statement` formatiga tushadi. Qo'shimchasiga Frontend tomondan berilayotgan barcha operatorlar tekshiriladi va xatar chaqirilsa qattiq rad etiladi.  
