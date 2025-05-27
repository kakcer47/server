from fastapi import FastAPI, HTTPException, Header
from fastapi.responses import StreamingResponse
from pydantic import BaseModel
import os
import json
import uvicorn
import hmac
import hashlib
from urllib.parse import parse_qs
import logging
from supabase import create_client, Client
import asyncio
from datetime import datetime

# Настройка логирования
logging.basicConfig(level=logging.INFO)
logger = logging.getLogger(__name__)

app = FastAPI()

# Подключение к Supabase
try:
    supabase: Client = create_client(
        os.getenv("SUPABASE_URL"),
        os.getenv("SUPABASE_KEY")
    )
    logger.info("Supabase подключён успешно")
except Exception as e:
    logger.error(f"Ошибка подключения к Supabase: {e}")
    raise

# Telegram Bot Token
TELEGRAM_BOT_TOKEN = os.getenv("TELEGRAM_BOT_TOKEN")
if not TELEGRAM_BOT_TOKEN:
    logger.warning("TELEGRAM_BOT_TOKEN не установлен")

class Ad(BaseModel):
    title: str
    description: str
    price: float
    user_id: str
    username: str
    timestamp: str

class Profile(BaseModel):
    user_id: str
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
    try:
        user_data = validate_init_data(auth.initData)
        if not user_data:
            raise HTTPException(status_code=401, detail="Invalid Telegram initData")
        return {"valid": True, "user": user_data}
    except Exception as e:
        logger.error(f"Ошибка верификации: {e}")
        raise HTTPException(status_code=500, detail="Ошибка сервера")

@app.post("/api/ads")
async def create_ad(ad: Ad, x_telegram_init_data: str = Header(None)):
    try:
        if not validate_init_data(x_telegram_init_data):
            raise HTTPException(status_code=401, detail="Invalid Telegram initData")
        ad_dict = ad.dict()
        response = supabase.table("ads").insert(ad_dict).execute()
        if response.data:
            logger.info(f"Объявление создано: {response.data[0]['id']}")
            return {"id": response.data[0]["id"]}
        else:
            raise HTTPException(status_code=500, detail="Ошибка создания объявления")
    except Exception as e:
        logger.error(f"Ошибка создания объявления: {e}")
        raise HTTPException(status_code=500, detail=str(e))

@app.get("/api/ads")
async def get_ads(before: str | None = None):
    try:
        query = supabase.table("ads").select("*").order("timestamp", desc=True).limit(20)
        if before:
            query = query.lt("timestamp", before)
        response = query.execute()
        logger.info(f"Загружено объявлений: {len(response.data)}")
        return response.data
    except Exception as e:
        logger.error(f"Ошибка загрузки объявлений: {e}")
        raise HTTPException(status_code=500, detail=str(e))

@app.post("/api/profile/{user_id}")
async def update_profile(user_id: str, profile: Profile, x_telegram_init_data: str = Header(None)):
    try:
        if not validate_init_data(x_telegram_init_data):
            raise HTTPException(status_code=401, detail="Invalid Telegram initData")
        response = supabase.table("profiles").upsert(
            {"user_id": user_id, "username": profile.username, "bio": profile.bio}
        ).execute()
        logger.info(f"Профиль обновлён: {user_id}")
        return {"status": "ok"}
    except Exception as e:
        logger.error(f"Ошибка обновления профиля: {e}")
        raise HTTPException(status_code=500, detail=str(e))

@app.get("/api/profile/{user_id}")
async def get_profile(user_id: str):
    try:
        response = supabase.table("profiles").select("*").eq("user_id", user_id).execute()
        profile = response.data[0] if response.data else {"bio": ""}
        logger.info(f"Профиль загружен: {user_id}")
        return profile
    except Exception as e:
        logger.error(f"Ошибка загрузки профиля: {e}")
        raise HTTPException(status_code=500, detail=str(e))

@app.get("/api/stream/ads")
async def stream_ads():
    async def event_generator():
        last_id = None
        while True:
            try:
                response = supabase.table("ads").select("*").gt("id", last_id or "").order("timestamp", desc=True).execute()
                for ad in response.data:
                    if not last_id or ad["id"] > last_id:
                        logger.info(f"Новое объявление через SSE: {ad['id']}")
                        yield f"data: {json.dumps(ad)}\n\n"
                        last_id = ad["id"]
                await asyncio.sleep(5)  # Проверка каждые 5 секунд
            except Exception as e:
                logger.error(f"Ошибка SSE: {e}")
                await asyncio.sleep(5)
    try:
        return StreamingResponse(event_generator(), media_type="text/event-stream")
    except Exception as e:
        logger.error(f"Ошибка инициализации SSE: {e}")
        raise HTTPException(status_code=500, detail="Ошибка сервера")

if __name__ == "__main__":
    port = int(os.getenv("PORT", 8000))
    uvicorn.run(app, host="0.0.0.0", port=port)
