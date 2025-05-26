from fastapi import FastAPI
from fastapi.responses import StreamingResponse
from pymongo import MongoClient
from pydantic import BaseModel
import os
import json
import uvicorn

app = FastAPI()

# Подключение к MongoDB Atlas
client = MongoClient(os.getenv("mongodb+srv://baza:<db_password>@cluster0.otzzusz.mongodb.net/classifieds?retryWrites=true&w=majority&appName=Cluster0"))
db = client["classifieds"]
ads_collection = db["ads"]
profiles_collection = db["profiles"]

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

@app.post("/api")
async def create_ad(ad: Ad):
    ad_dict = ad.dict()
    result = ads_collection.insert_one(ad_dict)
    return {"id": str(result.inserted_id)}

@app.get("/api")
async def get_ads():
    ads = list(ads_collection.find().sort("timestamp", -1).limit(50))
    for ad in ads:
        ad["_id"] = str(ad["_id"])
    return ads

@app.post("/api/profile/{user_id}")
async def update_profile(user_id: str, profile: Profile):
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
    return profile or {}

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
