from fastapi import FastAPI, Request
from pydantic import BaseModel
import httpx
import os
from fastapi.middleware.cors import CORSMiddleware

app = FastAPI()

app.add_middleware(
    CORSMiddleware,
    allow_origins=["https://ssw-livid.vercel.app"],  # или ["https://yourapp.vercel.app"]
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

BOT_TOKEN = "7948285859:AAGPM2BYYE2US3AIbP7P4yEBV4C5oWt3FSw"
CHAT_ID = "-1002361596586"

class MessageInput(BaseModel):
    username: str
    message: str

@app.post("/send")
async def send_message(data: MessageInput):
    text = f"📩 Сообщение от {data.username}:\n\n{data.message}"
    url = f"https://api.telegram.org/bot{BOT_TOKEN}/sendMessage"
    async with httpx.AsyncClient() as client:
        await client.post(url, data={"chat_id": CHAT_ID, "text": text})
    return {"status": "ok"}
