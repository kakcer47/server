from fastapi import FastAPI, HTTPException, Header
from fastapi.responses import StreamingResponse
from pymongo import MongoClient
from pydantic import BaseModel
import os
import json
import uvicorn
import hmac
import hashlib
from urllib.parse import parse_qs
import logging

# Настройка логирования
logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

app = FastAPI()

# Подключение к MongoDB Atlas
try:
    client = MongoClient(os.getenv("MONGODB_URI"))
    db = client["classifieds"]
    ads_collection = db["ads"]
    profiles_collection = db["profiles"]
    client.admin.command('ping')  # Проверка подключения
    logger.info("MongoDB подключён успешно")
except Exception as e:
    logger.error(f"Ошибка подключения к MongoDB: {e}")
    raise

# Telegram Bot Token
TELEGRAM_BOT_TOKEN = os.getenv("TELEGRAM_BOT_TOKEN")
if not TELEGRAM_BOT_TOKEN:
    logger.warning("TELEGRAM_BOT_TOKEN не установлен")

class Ad(BaseModel):
    title: str
    description: str
    price: float
    userId: str
    username: str
    timestamp: str

class Profile(BaseModel):
    userId: str
    username: str
    bio: str | None = None

class AuthRequest(BaseModel):
    initData: str

def validate_init_data(init_data: str) -> dict | None:
    if not TELEGRAM_BOT_TOKEN or not init_data:
        logger.warning("TELEGRAM_BOT_TOKEN или initData отсутствуют")
        return None
    try:
        parsed_data = parse_qs(init_data)
        hash_value = parsed_data.get("hash", [""])[0]
        data_check = "\n".join(
            f"{k}={v[0]}" for k, v in sorted(parsed_data.items()) if k != "hash"
        )
        secret_key = hmac.new(
            b"WebAppData", TELEGRAM_BOT_TOKEN.encode(), hashlib.sha256
        ).digest()
        computed_hash = hmac.new(
            secret_key, data_check.encode(), hashlib.sha256
        ).hexdigest()
        if not hmac.compare_digest(computed_hash, hash_value):
            logger.error("Недействительный хэш initData")
            return None
        user_data = json.loads(parsed_data.get("user", ["{}"])[0])
        logger.info(f"Валидация initData успешна: {user_data}")
        return user_data
    except Exception as e:
        logger.error(f"Ошибка валидации initData: {e}")
        return None

@app.post("/api/auth/verify")
async def verify_auth(auth: AuthRequest):
    user_data = validate_init_data(auth.initData)
    if not user_data:
        raise HTTPException(status_code=401, detail="Invalid Telegram initData")
    return {"valid": True, "user": user_data}

@app.post("/api/ads")
async def create_ad(ad: Ad, x_telegram_init_data: str = Header(None)):
    if not validate_init_data(x_telegram_init_data):
        raise HTTPException(status_code=401, detail="Invalid Telegram initData")
    try:
        ad_dict = ad.dict()
        result = ads_collection.insert_one(ad_dict)
        logger.info(f"Объявление создано: {result.inserted_id}")
        return {"id": str(result.inserted_id)}
    except Exception as e:
        logger.error(f"Ошибка создания объявления: {e}")
        raise HTTPException(status_code=500, detail="Ошибка сервера")

@app.get("/api/ads")
async def get_ads(before: str | None = None):
    try:
        query = {"timestamp": {"$lt": before}} if before else {}
        ads = list(ads_collection.find(query).sort("timestamp", -1).limit(20))
        for ad in ads:
            ad["_id"] = str(ad["_id"])
        logger.info(f"Загружено объявлений: {len(ads)}")
        return ads
    except Exception as e:
        logger.error(f"Ошибка загрузки объявлений: {e}")
        raise HTTPException(status_code=500, detail="Ошибка сервера")

@app.post("/api/profile/{user_id}")
async def update_profile(user_id: str, profile: Profile, x_telegram_init_data: str = Header(None)):
    if not validate_init_data(x_telegram_init_data):
        raise HTTPException(status_code=401, detail="Invalid Telegram initData")
    try:
        profiles_collection.update_one(
            {"userId": user_id},
            {"$set": {"username": profile.username, "bio": profile.bio}},
            upsert=True
        )
        logger.info(f"Профиль обновлён: {user_id}")
        return {"status": "ok"}
    except Exception as e:
        logger.error(f"Ошибка обновления профиля: {e}")
        raise HTTPException(status_code=500, detail="Ошибка сервера")

@app.get("/api/profile/{user_id}")
async def get_profile(user_id: str):
    try:
        profile = profiles_collection.find_one({"userId": user_id})
        if profile:
            profile["_id"] = str(profile["_id"])
            logger.info(f"Профиль загружен: {user_id}")
        else:
            logger.info(f"Профиль не найден: {user_id}")
        return profile or {"bio": ""}
    except Exception as e:
        logger.error(f"Ошибка загрузки профиля: {e}")
        raise HTTPException(status_code=500, detail="Ошибка сервера")

@app.get("/api/stream/ads")
async def stream_ads():
    async def event_generator():
        try:
            async with ads_collection.watch() as stream:
                async for change in stream:
                    if change["operationType"] == "insert":
                        ad = change["fullDocument"]
                        ad["_id"] = str(ad["_id"])
                        logger.info(f"Новое объявление через SSE: {ad['_id']}")
                        yield f"data: {json.dumps(ad)}\n\n"
        except Exception as e:
            logger.error(f"Ошибка SSE: {e}")
    try:
        return StreamingResponse(event_generator(), media_type="text/event-stream")
    except Exception as e:
        logger.error(f"Ошибка инициализации SSE: {e}")
        raise HTTPException(status_code=500, detail="Ошибка сервера")

if __name__ == "__main__":
    port = int(os.getenv("PORT", 8000))
    uvicorn.run(app, host="0.0.0.0", port=port)
