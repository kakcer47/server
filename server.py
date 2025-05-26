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

app = FastAPI()

# Подключение к MongoDB Atlas
client = MongoClient(os.getenv("MONGODB_URI"))
db = client["classifieds"]
ads_collection = db["ads"]
profiles_collection = db["profiles"]

# Telegram Bot Token
TELEGRAM_BOT_TOKEN = os.getenv("TELEGRAM_BOT_TOKEN")

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

def validate_init_data(init_data: str) -> bool:
    if not TELEGRAM_BOT_TOKEN or not init_data:
        return True  # Для локального тестирования
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
        return hmac.compare_digest(computed_hash, hash_value)
    except Exception as e:
        print(f"Ошибка валидации initData: {e}")
        return False

@app.post("/api/ads")
async def create_ad(ad: Ad, x_telegram_init_data: str = Header(None)):
    if not validate_init_data(x_telegram_init_data):
        raise HTTPException(status_code=401, detail="Invalid Telegram initData")
    ad_dict = ad.dict()
    result = ads_collection.insert_one(ad_dict)
    return {"id": str(result.inserted_id)}

@app.get("/api/ads")
async def get_ads(before: str | None = None):
    query = {"timestamp": {"$lt": before}} if before else {}
    ads = list(ads_collection.find(query).sort("timestamp", -1).limit(20))
    for ad in ads:
        ad["_id"] = str(ad["_id"])
    return ads

@app.post("/api/profile/{user_id}")
async def update_profile(user_id: str, profile: Profile, x_telegram_init_data: str = Header(None)):
    if not validate_init_data(x_telegram_init_data):
        raise HTTPException(status_code=401, detail="Invalid Telegram initData")
    profiles_collection.update_one(
        {"userId": user_id},
        {"$set": {"username": profile.username, "bio": profile.bio}},
        upsert=True
    )
    return {"status": "ok"}

@app.get("/api/profile/{user_id}")
async def get_profile(user_id: str):
    profile = profiles_collection.find_one({"userId": user_id})
    if profile:
        profile["_id"] = str(profile["_id"])
    return profile or {"bio": ""}

@app.get("/api/stream/ads")
async def stream_ads():
    async def event_generator():
        async with ads_collection.watch() as stream:
            async for change in stream:
                if change["operationType"] == "insert":
                    ad = change["fullDocument"]
                    ad["_id"] = str(ad["_id"])
                    yield f"data: {json.dumps(ad)}\n\n"
    return StreamingResponse(event_generator(), media_type="text/event-stream")

if __name__ == "__main__":
    port = int(os.getenv("PORT", 8000))
    uvicorn.run(app, host="0.0.0.0", port=port)
