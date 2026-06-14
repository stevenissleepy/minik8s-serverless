import json
import socket
import time


def new():
    return Function()


class Function:
    async def handle(self, scope, receive, send):
        message = await receive()
        body = message.get("body", b"")
        event = json.loads(body.decode("utf-8")) if body.strip() else {}
        text = str(event.get("text", "")).lower()
        category = event.get("category", "general")
        user_level = event.get("user_level", "normal")
        sleep_ms = int(event.get("sleep_ms", 0) or 0)
        if sleep_ms > 0:
            time.sleep(sleep_ms / 1000)

        risk = 25
        if user_level == "vip":
            risk += 15
        if category == "refund":
            risk += 25
        if category == "complaint":
            risk += 30
        for word in ["broken", "urgent", "angry", "terrible", "security", "leak"]:
            if word in text:
                risk += 15
        risk = min(risk, 100)

        result = {
            **event,
            "risk": risk,
            "risk_level": "high" if risk >= 80 else "normal",
            "decision": "human" if risk >= 80 else "auto",
            "model_version": "risk-v2",
            "instance": socket.gethostname(),
            "scored_by": scope["minik8s"]["name"],
        }
        await respond(send, result)


async def respond(send, result):
    payload = json.dumps(result).encode("utf-8")
    await send({
        "type": "http.response.start",
        "status": 200,
        "headers": [[b"content-type", b"application/json"]],
    })
    await send({"type": "http.response.body", "body": payload})
