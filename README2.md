# Universal Adaptive Query (UAQ) Engine

**Rust** tilida yozilgan, FFI orqali har qanday dasturlash tili bilan ishlaydigan **JSON → PostgreSQL** kompilyatori.

Frontend deklarativ JSON yuboradi → Engine xavfsiz SQL hosil qiladi → Backend uni bazaga yuboradi.

---

## Tezkor Boshlash (PHP)

```php
$ffi = \FFI::cdef("
    char* uaq_parse(const char* json_input, const char* whitelist, const char* relations, const char* macros_input);
    void uaq_free_string(char* s);
", __DIR__ . '/libjson_to_sql.so');

$raw    = $ffi->uaq_parse($json, $whitelist, $relations, null);
$result = json_decode(\FFI::string($raw), true);
$ffi->uaq_free_string($raw);

// $result['isOk'] === true bo'lsa, $result['sql'] va $result['params'] tayyor
```

---

## Asosiy Funksiya

### `uaq_parse` — SQL Generator

```c
char* uaq_parse(
    const char* json_input,     // Frontend so'rovi
    const char* whitelist_json, // Ruxsat etilgan jadvallar va ustunlar
    const char* relations_json, // Jadvallar orasidagi JOIN yo'llari
    const char* macros_json     // Oldindan belgilangan shablonlar (ixtiyoriy, null bo'lishi mumkin)
);
```

**Muvaffaqiyatli javob:**
```json
{ "isOk": true, "sql": "SELECT ...", "params": { "p1": "1" }, "message": "success" }
```

**Xatolik javob:**
```json
{ "isOk": false, "sql": null, "params": null, "message": "Xato matni..." }
```

### `uaq_free_string` — Xotirani Tozalash

```c
void uaq_free_string(char* s);
```

`uaq_parse` qaytargan har bir string uchun chaqirish **majburiy** — aks holda memory leak yuzaga keladi.

---

## Frontend So'rovi (JSON Formati)

Root kalit uchta variantdan biri bo'lishi mumkin:

| Kalit     | Vazifasi                                          |
|-----------|---------------------------------------------------|
| `@data`   | Bitta obyekt qaytaradi `{...}`                    |
| `@data[]` | Massiv qaytaradi `[{...}, {...}]`                 |
| `@info`   | Tizim strukturasi (jadvallar, relationlar) haqida |

### Direktivalar

| Direktiva  | Vazifasi                                         | Majburiy |
|------------|--------------------------------------------------|----------|
| `@source`  | Manba jadval, filtrlar va konfiguratsiya         | Ha       |
| `@fields`  | Qaytariladigan maydonlar va ularni nomlash       | Yo'q     |
| `@flatten` | Bola node maydonlarini ota nodega qo'shib beradi | Yo'q     |
| `@extend`  | Makrosni kengaytirish                            | Yo'q     |
| `[]`       | Kalit oxiriga qo'shilsa — massiv qaytaradi       | Yo'q     |

---

## `@source` Sintaksisi

```
jadval_alias[maydon: qiymat, ..., $limit: N, $offset: N, $order: ustun DIR, $join: tip]
```

### Filtr operatorlari

| Operator | Ma'nosi   | Misol                   |
|----------|-----------|-------------------------|
| `:`      | Teng      | `status: 1`             |
| `!:`     | Teng emas | `type: !: 0`            |
| `>`      | Katta     | `age: > 18`             |
| `<`      | Kichik    | `age: < 65`             |
| `..`     | Oraliq    | `id: 1..100`            |
| `~`      | LIKE      | `name: ~ Ali%`          |
| `in`     | Ro'yxatda | `rank: in (1, 2, 3)`    |

### Konfiguratsiya parametrlari

| Parametr  | Misol              | Izoh                               |
|-----------|--------------------|------------------------------------|
| `$limit`  | `$limit: 20`       | Qaytariladigan qatorlar soni       |
| `$offset` | `$offset: 40`      | Nechta qatorni o'tkazib yuborish   |
| `$order`  | `$order: id DESC`  | Tartiblash ustuni va yo'nalishi    |
| `$join`   | `$join: left`      | JOIN turini qo'lda belgilash       |
| `$rel`    | `$rel: emp_admin`  | Aniq relation nomini ko'rsatish    |

---

## So'rov Namunalari

### Oddiy ro'yxat

```json
{
  "@data[]": {
    "@source": "emp[status: 1, $limit: 20, $order: id DESC]",
    "@fields": ["id", "full_name"]
  }
}
```

### Bitta obyekt + sana formatlash

```json
{
  "@data": {
    "@source": "emp[status: 1, id: 42]",
    "@fields": {
      "id": "id",
      "full_name": "CONCAT(last_name, ' ', first_name)",
      "tug_ilgan_kun": "TO_CHAR(TO_TIMESTAMP(birthday), 'DD.MM.YYYY')"
    }
  }
}
```

### Ichma-ich massiv + flatten

```json
{
  "@data": {
    "@source": "emp[status: 1, id: 42]",
    "@fields": { "id": "id", "full_name": "full_name" },

    "positions[]": {
      "@source": "department_staff_position[is_current: 1, $limit: 5]",
      "@fields": { "id": "id", "start_date": "TO_CHAR(TO_TIMESTAMP(start_time), 'DD.MM.YYYY')" },

      "position_info": {
        "@source": "staff_position[status: 1]",
        "@flatten": true,
        "@fields": { "name": "name_uz" }
      }
    },

    "degree": {
      "@source": "military_degree[current_degree: 1]",
      "@flatten": true,
      "@fields": { "degree_name": "name_uz" }
    }
  }
}
```

> `@flatten: true` — bola node maydonlarini ota nodega qo'shib yuboradi, alohida ichki obyekt yaratilmaydi.

### Tizim strukturasini o'qish

```json
{ "@info": ["@tables", "@relations"] }
```

---

## Backend Parametrlari

### 1. Whitelist — Xavfsizlik Qatlami

Frontendga qaysi jadval va ustunlarni ko'rsatish mumkinligini belgilaydi. Format: `"haqiqiy_jadval:alias"`.

**Oddiy (ustunlar ro'yxati):**
```json
{
  "employee:emp": ["id", "first_name", "last_name", "status", "birthday"],
  "education:edu": ["*"]
}
```

**Mapping (DB arxitekturasini yashirish va SQL ifodalar):**
```json
{
  "structure_organization:org": {
    "unique":     "id",
    "title":      "name_uz",
    "active":     "status",
    "full_name":  "CONCAT(last_name, ' ', first_name)"
  }
}
```

> Frontend `org[active: 1]` yozadi → Engine `org.status = :p1` ga o'giradi. Haqiqiy ustun nomi yashirinadi.

---

### 2. Relations — Avtomatik JOIN

Jadvallar orasidagi bog'lanishni bir marta yozasiz. Engine so'rovda kerak bo'lganda avtomatik JOIN quradi.

**Yo'nalish operatorlari:**

| Kalit  | JOIN turi  |
|--------|------------|
| `->`   | LEFT JOIN  |
| `<-`   | RIGHT JOIN |
| `-><-` | INNER JOIN |
| `<->`  | FULL JOIN  |

```json
{
  "emp->emp_rel_org": "@join @table ON @2.employee_id = @1.id",
  "emp_rel_org->org": "@join @table ON @2.id = @1.organization_id AND @1.status = 1"
}
```

**Placeholder ma'nolari:**

| Placeholder | Ma'nosi                           |
|-------------|-----------------------------------|
| `@join`     | Join turi SQL nomi                |
| `@table`    | Child jadvalning haqiqiy SQL nomi |
| `@1`        | Kalitnng birinchi alias           |
| `@2`        | Kalitning ikkinchi alias          |

**Avtomatik yo'l (BFS):** `emp → org` to'g'ridan-to'g'ri relation bo'lmasa ham, Engine oraliq jadvallar orqali (`emp → emp_rel_org → org`) yo'lni o'zi topadi va barcha oraliq jadvallarni JOIN qiladi.

---

### 3. Macros — Qayta Ishlatiluvchi Shablonlar

Tez-tez chaqiriladigan murakkab so'rovlarni oldindan yozib, `@extend` yoki `@source` orqali ishlatish mumkin.

**Ta'rif (backendda):**
```json
{
  "activeEmployee": {
    "@source": "emp[status: 1]",
    "@fields": { "id": "id", "full_name": "CONCAT(last_name, ' ', first_name)" }
  }
}
```

**Oddiy chaqiruv:**
```json
{
  "@data[]": { "@extend": "activeEmployee" }
}
```

**Qo'shimcha filtr bilan kengaytirish:**
```json
{
  "@data[]": {
    "@source": "activeEmployee[$limit: 5, $order: id DESC]"
  }
}
```

---

## Xavfsizlik

- Barcha filtr qiymatlari `:p1`, `:p2` shaklida **parametrlashtirilib** SQL injection imkoniyati yo'q.
- Frontend faqat **whitelist**da ruxsat etilgan jadval va ustunlarga murojaat qila oladi.
- `DROP`, `DELETE`, `--`, `/* */` kabi xavfli konstruktsiyalar **qat'iy bloklanadi**.
- SQL funksiyalaridan faqat ruxsat etilganlari (`CONCAT`, `TO_CHAR`, `COALESCE`, `CASE WHEN` va boshqalar) qabul qilinadi.

---

## Boshqa Tillar Bilan Integratsiya

| Platforma | Fayl    | Usul                |
|-----------|---------|---------------------|
| PHP       | `.so`   | FFI                 |
| Python    | `.so`   | `ctypes` / `cffi`   |
| Node.js   | `.wasm` | WebAssembly         |
| Java      | `.so`   | JNI / Project Panama|
| Go        | `.so`   | `cgo`               |

**C-Header:**
```c
char* uaq_parse(const char* json_input, const char* whitelist, const char* relations, const char* macros_input);
void uaq_free_string(char* s);
```

---

## Qo'shimcha Imkoniyat: Fayl Ma'lumotlarini Base64 ga Aylantirish

Ayrim hollarda (loyihalar arasi integratsiya, bitta yozuv uchun bir martali eksport va h.k.) bazadan kelgan JSON ichidagi fayl manzillarini bevosita **Base64** ko'rinishiga o'girish kerak bo'ladi. Buning uchun kutubxonada alohida yordamchi funksiya mavjud.

### `uaq_inject_base64_files`

```c
char* uaq_inject_base64_files(
    const char* json_result,     // Bazadan kelgan tayyor JSON string
    const char* root_files_path, // Fayllar joylashgan asosiy papka: "/var/www/project"
    const char* trigger_prefix   // Qaysi qiymatlarni aylantirish: "/web/uploads/"
);
```

Funksiya JSON ichini rekursiv aylanib chiqadi. `trigger_prefix` bilan boshlanadigan har bir string qiymatni topib, `root_files_path + qiymat` yo'li bo'yicha faylni diskdan o'qiydi va `data:image/jpeg;base64,...` ko'rinishiga o'giradi.

**Qo'llab-quvvatlanadigan MIME turlar:** `jpg/jpeg`, `png`, `gif`, `webp`, `svg`, `pdf`, `mp4` va boshqalar.

### PHP da ishlatish

```php
// FFI ta'rifiga qo'shimcha qiling:
$ffi = \FFI::cdef("
    char* uaq_parse(const char* json_input, const char* whitelist, const char* relations, const char* macros_input);
    char* uaq_inject_base64_files(const char* json_result, const char* root_files_path, const char* trigger_prefix);
    void uaq_free_string(char* s);
", __DIR__ . '/libjson_to_sql.so');

// 1. Avval SQL oling va bazaga yuboring → $dbJsonString

// 2. Fayl manzillarini Base64 ga aylantiring
$raw   = $ffi->uaq_inject_base64_files($dbJsonString, '/var/www/my-project', '/web/uploads/');
$final = json_decode(\FFI::string($raw), true);
$ffi->uaq_free_string($raw);
```

**Natija:** JSON ichidagi `"/web/uploads/pasport.pdf"` → `"data:application/pdf;base64,JVBERi0x..."`

> **Qachon ishlatish kerak:** Loyihalar arasi bir martali ma'lumot uzatishda (narigi tomon fayllarni o'z serveriga saqlashi kerak bo'lganda).  
> **Qachon ishlatmaslik kerak:** Umumiy ro'yxat endpointlarida. Base64 fayl hajmini ~33% kattalashtiradi va brauzer keshi ishlamaydi.
