from fastapi import FastAPI
from fastapi.middleware.cors import CORSMiddleware
from pydantic import BaseModel
import requests

app = FastAPI()  # ✅ вот здесь создаём объект приложения

# Разрешаем CORS с Vercel-домена
app.add_middleware(
    CORSMiddleware,
    allow_origins=["https://ssw-livid.vercel.app"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

# Модель запроса
class Message(BaseModel):
    username: str
    message: str

# Роут отправки
@app.post("/send")
def send_message(msg: Message):
    text = f"✉️ Новое сообщение от {msg.username}:\n{msg.message}"
    url = "https://api.telegram.org/bot7948285859:AAGPM2BYYE2US3AIbP7P4yEBV4C5oWt3FSw/sendMessage"
    payload = {
        "chat_id": "-1002361596586",
        "text": text
    }
    r = requests.post(url, data=payload)
    return {"ok": True, "status": r.status_code}
