# Universal Adaptive Query (UAQ) Engine

UAQ (Universal Adaptive Query) bu front-end va back-end orasidagi ma'lumotlar almashinuvini butunlay yangi darajaga olib chiquvchi, yuqori samaradorlikka ega **JSON-to-SQL kompilyatori** hisoblanadi.

Ushbu kutubxona (Rustda yozilgan) yordamida frontend dasturchilar shunchaki o'zlariga kerakli ma'lumotlar strukturasini JSON ko'rinishida yuboradilar. UAQ esa uni o'ta tezkor tekshiruvdan o'tkazib (Whitelist), bevosita murakkab va optimizatsiya qilingan (LATERAL JOIN) PostgreSQL so'rovlariga o'girib beradi. Natijada Backend ortiqcha `Controller` yoki `Service` mantiqlarini yozishdan to'liq xalos bo'ladi, Front-end esa juda moslashuvchan muhitga ega bo'ladi.

---

## 🏛 Arxitektura

UAQ 3 ta asosiy qismdan iborat:
1. **Frontend Input JSON (`query`)**: Frontend qanday shakldagi ma'lumotlarni qanday shartlar asosida olmoqchi ekanligini bildiruvchi fayl.
2. **Backend Whitelist (`whitelist`)**: Qaysi jadvallarga ruxsat borligi va qaysi alias-lar ortida aslida nima yashiringanini ko'rsatib beruvchi qat'iy xavfsizlik (Security) qatlami.
3. **Backend Macros (`macros_json`)**: (Ixtiyoriy) Frontend uchun ishlashni super osonlashtirish uchun Backend tomonidan tayyorlab beriladigan shablonlar (Virtual Ctes / Tables).

---

## 👨‍💻 Backend Dasturchilar Uchun Qo'llanma

Backend dasturchi UAQ ni ishga tushirish uchun ushbu kutubxona (`libjson_to_sql.so`) ni loyihaga ulashi va 4 ta asosiy parametrni berishi kerak.

### 1. API ni Chaqirish (C FFI)
Kutubxona C-FFI orqali ishlaydi (PHP/Node.js/Python kabi tillar uchun moslashtirilgan):

```c
char* uaq_parse(
    const char* json_input,     // Frontenddan kelgan JSON
    const char* whitelist_json, // Xavfsizlik qoidalari
    const char* relations_json, // Jadvallar orasidagi bog'lanish (Foreign keys)
    const char* macros_json     // Ixtiyoriy: Oldindan tayyorlangan macro shablonlar
);
```

### 2. Whitelist Tuzish
Backend tizimdagi qaysi jadval va ustunlarga ruxsat berishni hal qiladi:
*Eslatma:* Agar massiv `["*"]` berilsa o'sha jadvalning barcha ustunlariga ruxsat etiladi. Obyekt berilsa faqat aytilganlari o'tadi.

```json
{
  "employee:emp": ["*"],
  "structure_organization:org": {
    "unique": "id",
    "name": "name_uz",
    "status": "status"
  }
}
```

### 3. Relations Tuzish
Jadvallarning qaysi ustun orqali ulanishini bildiradi:

```json
{
  "emp->org": "emp.org_id = org.id",
  "departmentStaffPosition->emp": "departmentStaffPosition.employee_id = emp.id AND departmentStaffPosition.status = 1"
}
```

### 4. Macros (Virtual Shablonlar) yaratish
Agar ma'lumotlar bazasida relatsiyalar (Joinlar) judayam chuqur bo'lib ketsa, Backend uni chiroyli qilib paketlab Macro qilib frontendga taqdim eta oladi. Frontendchi uni oddiy jadval sifatida ishlatadi:

```json
{
  "positionCteTable": {
    "@source": "departmentStaffPosition[is_current: true]",
    "@fields": ["*"],
    "test": {
      "@source": "org[status: 1]",
      "@flatten": true, 
      "@fields": {
        "viloyat_boshqarma": "name", 
        "org_maqomi": "status"
      }
    }
  }
}
```
*`@flatten: true` funksiyasi — Ichkaridagi bolaning (`org`) ustunlarini ham huddi ota (`departmentStaffPosition`) bilan birga chiqqandek tekis (flat) qilib yuboradi*.

---

## 🎨 Frontend Dasturchilar Uchun Qo'llanma

Frontend dasturchilar API-ga faqatgina bitta parametr integratsiya qiladilar: `JSON Query`.

Ushbu ko'rinishda siz xuddi GraphQL dagi singari kerakli ma'lumotlarni daraxt (tree) ko'rinishida so'raysiz. Eng asosiysi, hamma murakkab bog'liqliklarni Backend Whitelist va Relations ichida hal qilib bo'lganligi sababli, frontend shunchaki so'rash bilan band bo'ladi.

### 1. Eng Oddiy So'rov (Barchasini olish)

Agar ro'yxat kerak bo'lsa `@data[]` ishlatiladi, bitta obyekt kerak bo'lsa `@data`.

```json
{
  "@data[]": {
    "@source": "emp[status: 1, $limit: 20, $order: id DESC]",
    "@fields": ["*"]
  }
}
```
*Tushuntirish:* Barcha statusi 1 ga teng bo'lgan employee (emp) larni oxirgi qo'shilganidan boshlab 20 tasini olib ber. `@fields: ["*"]` jami bor ustunlarni ifodalaydi.

### 2. Aynan kerakli maydonlarni olish + Maxsus SQL funksiyalar (CONCAT)

DB dagi haqiqiy ustunlarni moslashtirish, qo'shish va maxsus ismlar bilan almashtirish (Aliasing):

```json
{
  "@data[]": {
    "@source": "emp[id: 1000..2000]",
    "@fields": {
      "user_id": "id",
      "full_name": "CONCAT(last_name, ' ', first_name)",
      "tugilgan_yil": "TO_CHAR(TO_TIMESTAMP(birthday), 'YYYY')"
    }
  }
}
```
***Muhim qoida!*** 
* Agar ustunlarga **Massiv** (`["*"]`) ishlatsangiz: Asl ustunlar qanday bo'lsa barchasi shunday keladi.
* Agar ustunlarga **Obyekt** (`{"aliasi": "asl_ustuni"}`) ishlatsangiz: Qat'iy nazorat yoqiladi! Massiv yo'qqa chiqib faqat va faqat o'zingiz yozgan qatorlargina JSON-da aks etadi.

### 3. Ichma-ich Ma'lumotlarni (Relations / Nested) So'rash

Agar employee ning ichida unga ulangan pozitsiyalarini ham so'rasangiz, ro'yxat shaklida kelishi uchun "[]" bilan yangi obyekt ochasiz:

```json
{
  "@data[]": {
    "@source": "emp",
    "@fields": ["*"],
    "positions[]": {
      "@source": "departmentStaffPosition[is_current: true]",
      "@fields": {
         "start_date": "start_time"
      }
    }
  }
}
```

### 4. Macro-CTElardan Foydalanish

Backend yaratib bergan murakkab daraxtni siz shunchaki oddiy jadval nomidek `@source` ichida chaqirib ketaverasiz. Uni oddiy tablelardan deyarli farqi yo'q:

```json
{
  "@data[]": {
    "@source": "emp",
    "@fields": ["*"],
    "position": {
      "@source": "positionCteTable",
      "@fields": {
         "full_position": "CONCAT(viloyat_boshqarma, ' ', tuman_boshqarma)"
      }
    }
  }
}
```
Tizim judayam Intellektual ishlaydi:
Yuqoridagi so'rovda siz faqat `"full_position": "..."` deb so'radingiz va **"*"** qatnashmadi, shu sababli ham tizim sizga kerakmas ma'lumotlarni bermaydi (id, start_time kabi) va faqat o'ziga yozilgan ma'lumotlar bilan javobni ixcham qaytaradi:

**Backend Qaytaradigan Javob (Response):**
```json
{
  "full_name": "Irgashev Dilshod",
  "id": 2145,
  "status": 1,
  "position": {
    "full_position": "Ichki ishlar vazirligi Transport boshqarmasi... Buxoro tuman bo'limi..."
  }
}
```

#### Barcha Xususiyatlari:
- Filtrlash turlari: `= val`, `!:` (Teng emas), `<` (Kichik), `>` (Katta), `~` (LIKE izlash), `id: 1..100` (Between oraliq), `in (1, 2, 3)`.
- Cheklovlar: `$limit: 20`, `$offset: 0`, `$order: id DESC`.
- Bog'lanish turini o'zgartirish: `$join: LEFT`.

---

**Xavfsizlik**
Frontend qancha murakkab SOQL yozmasin, Backend tomonidan Whitelistga kiritilmagan bo'lsa "Generation Error: Column 'x' does not exist" xatosini oladi. Tizim to'lig'icha "SQL Injection" hujumlaridan himoyalangan.
