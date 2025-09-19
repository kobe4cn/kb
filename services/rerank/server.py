from fastapi import FastAPI, Header, HTTPException
from pydantic import BaseModel
from typing import List
from sentence_transformers import CrossEncoder
import os
import dotenv
import uvicorn

app = FastAPI()
dotenv.load_dotenv()
model = CrossEncoder('cross-encoder/ms-marco-MiniLM-L-12-v2')
RERANK_TOKEN = os.getenv('RERANK_TOKEN')

class RerankReq(BaseModel):
    query: str
    candidates: List[str]

class RerankResp(BaseModel):
    scores: List[float]

@app.get('/health')
def health():
    return {"status": "ok"}

@app.post('/rerank', response_model=RerankResp)
def rerank(req: RerankReq, authorization: str | None = Header(default=None)):
    if RERANK_TOKEN:
        if not authorization or authorization != f"Bearer {RERANK_TOKEN}":
            raise HTTPException(status_code=401, detail="unauthorized")
    pairs = [(req.query, c) for c in req.candidates]
    scores = model.predict(pairs).tolist()
    return RerankResp(scores=scores)

if __name__ == "__main__":
    uvicorn.run(app, host="0.0.0.0", port=8000)
