# Universal Adaptive Query (UAQ) Engine

UAQ (Universal Adaptive Query) — bu tizim qismlari oralig'idagi ma'lumotlar ma'muriyatini osonlashtirish uchun yozilgan, yuqori tezlikka ega JSON-to-SQL kompilyatori.
Kutubxona xavfsiz C-FFI (Rust) ustiga qurilgan bo'lib, kiruvchi JSON formatidagi dinamik so'rovlarni xavfsizlik (Whitelist) qoidalariga asosan tekshiradi va PostgreSQL uchun optimizatsiya qilingan (LATERAL JOIN) SQL so'rovlariga aylantiradi.

---

## 1. Tizim Arxitekturasi

UAQ ishlashi uchun 4 ta asosiy qatlam mavjud:
1. **JSON Input**: Frontend yuboradigan daraxt (tree) ko'rinishidagi ma'lumot qidiruv shakli.
2. **Whitelist**: Qaysi jadval va ustunlarga ruxsat borligini nazorat qiluvchi Security qatlami.
3. **Relations**: Jadvallararo avtomatik ulash (Join) mexanizmini belgilab beruvchi Graph qatlami.
4. **Macros**: (Ixtiyoriy) Oldindan yaratilgan abstrakt shablonlar / Virtual jadvallar to'plami.

---

## 2. Backend Integratsiyasi

### C-FFI API
Kutubxona barcha mashhur tillar (Java (JNA), PHP (FFI), Node.js, Python, Go, C#) bilan quyidagi funksiya va xotirani tozalash API'si orqali ishlaydi:
```c
char* uaq_parse(
    const char* json_input,     // Frontend so'rovi
    const char* whitelist_json, // Ruxsat etilgan maydonlar va Aliaslar
    const char* relations_json, // Baza bo'yicha foreign key bog'lamalari
    const char* macros_json     // Ixtiyoriy: Macro-shablonlar
);

void uaq_free_string(char* s);  // Xotirada qolib ketmasligi uchun tozalash
```

### Whitelist Konfiguratsiyasi (Security & Mapping)
Format: `"database_table_name:frontend_table_name"`
Massiv `["*"]` berilsa so'zingsiz barcha ustunlarga ruxsat etiladi, Obyekt berilsa faqatgina xaritaga olingan ko'rsatkichlarga ruxsat beriladi va `CONCAT` kabi SQL agregatlar ruxsatdan o'tadi.
```json
{
  "user_profile:profiles": ["*"],
  "employee:emp": {
    "id": "id",
    "full_name": "CONCAT(last_name, ' ', first_name)",
    "status": "status"
  },
  "organization:org": [
    "id",
    "name",
    "status"
  ],
  "department:dep": [
    "id",
    "name",
    "status"
  ],
  "staff_position:position": ["*"]
}
```

### Relations Konfiguratsiyasi (Joins)
Jadvallarni bir-biriga tushuntirish uchun xaritalash:
```json
{
  "emp->profiles": "emp.profile_id = profiles.id AND profiles.is_active = 1"
}
```

### Macros Konfiguratsiyasi (Virtual Tables)
Murakkab ierarxiyani yagona jadval sifatida e'lon qilish:
```json
{
  "virtualPosition": {
    "@source": "position[is_active: true]",
    "@fields": ["*"],
    "info": {
      "@source": "org[status: 1]",
      "@flatten": true,
      "@fields": { "organization_name": "name" },
      "department": {
        "@source": "dep[status: 1]",
        "@flatten": true,
        "@fields": { "department_name": "name" }
      }
    }
  }
}
```
*Izoh:* `@flatten: true` - farzand jadvaldan keladigan ma'lumotlarni alohida obyekt qilib emas, ota jadvalning (employee_info) o'ziga qo'shib yuboradi.

---

## 3. Frontend Imkoniyatlari (JSON Query)

Frontend SQL yoki backenddagi murakkab relatsiyalarni bilmagan holatda so'rovlarni avtomatik qura oladi.

### Asosiy Parametrlar va Diktatorlar
- `@data` - Bitta ma'lumot obyektini tortib olish.
- `@data[]` - Ma'lumotlarni massiv (ro'yxat) qilib olish.
- `@source` - Jadval nomi (yoki Macro nomi) va unga tegishli Filtering/Sorting shartlari.
- `@fields` - Aynan qaysi ustunlar qaytishi kerakligi (Turi: Array yoki Object).
- `$limit`, `$offset`, `$order` - Paginate va saralash uchun maxsus keywordlar.
- `$join` - Ulash turini (Masalan: `LEFT`) o'zgartirish.

### So'rov namunalari

**1. Oddiy Ma'lumot (Array Fields)**
Jadvaldagi istalgan xavfsiz ruxsat etilgan maydonlarni massiv formatida so'rash:
```json
{
  "@data[]": {
    "@source": "emp",
    "@fields": ["id", "status"]
  }
}
```

**2. Asosiy Filtrlar (Where Clauses)**
`@source` ichida massiv shaklida filtrlar argument qilib olinishi mumkin:
- `=`: `status: 1`
- `!=`: `type: !:0`
- `>`, `<`: `age: >18`, `price: <500`
- `LIKE`: `name: ~John`
- `IN`: `category: in (1, 2, 3)`
- `BETWEEN`: `id: 10..50`

```json
"@source": "emp[status: 1, age: >25, type: in (1, 2), id: 1..100]"
```

**3. Nested Fetching (Bog'langan Jadvallarni olish)**
Relations configuration-da ulab qo'yilgan bo'lsa, o'zaro nom chaqirilishi kifoya. (Ro'yxat bo'lsa `[]` qo'shiladi):
```json
{
  "@data": {
    "@source": "profiles[id: 12, is_active: true]",
    "@fields": ["*"],
    "employee_info": {
      "@source": "emp[status: 1]",
      "@fields": { "fullName": "full_name" },
      "organization": {
        "@source": "org[status: 1]",
        "@flatten": true,
        "@fields": { "name": "name" }
      }
    }
  }
}
```

**4. Obyekt (Strict Overwrite) va Custom SQL Funksiyalar**
`@fields` kaliti ichiga oddiy massiv o'rniga Obyekt ifodalansa u strikt rejimida ishlaydi, ya'ni faqat o'zi aytgan so'rovni chiqaradi. Uning ichida avtomatik o'zgaruvchilarga asoslangan Funksiyalar ifodalash mumkin:
```json
{
  "@data": {
    "@source": "emp",
    "@fields": {
      "full_name_from_frontend": "CONCAT(first_name, ' ', last_name)",
      "custom_date": "TO_CHAR(TO_TIMESTAMP(created_at), 'DD.MM.YYYY')"
    }
  }
}
```

---

## 4. Macro Filtering (Overriding) xususiyati

Agar Macro ichida oldindan yozib qo'yilgan Array fieldlar (`@fields: ["*"]`) kelsa-yu, Frontend ularni ishlatish uchun qayta murojaat qilsa, tizim gibrid-konvergensiya qilmaydi. Agar Obyekt orqali so'ralsa barchasi Qat'iy O'chish (Overwrite) orqali shakllanadi va Frontend NIMA SO'RASA SHUNI OLADI.

**Misol:**
```json
{
  "@data": {
    "position": {
      "@source": "virtualPosition",
      "@fields": {
         "my_custom_concat": "CONCAT(organization_name, ' - ', department_name)"
      }
    }
  }
}
```
**Natija:**
Mantiq bo'yicha oldindan `id`, `status`, `is_current` chiqishi va uning orqasidan `organization_name` va `department_name` ham flatten orqali javobga birikib ketishi belgilangan bo'lsa-da, Frontend `@fields` ga obyekti bilan aralashgani uchun faqatgina **`my_custom_concat`** ni oladi, eski ma'lumotlar e'tiborsiz qoldiriladi. Tizimda keraksiz ma'lumot ("over-fetching") bo'lmaydi.
