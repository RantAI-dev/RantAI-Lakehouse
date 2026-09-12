FROM python:3.12-slim
RUN pip install --no-cache-dir "pyjwt[crypto]==2.9.0" cryptography==43.0.1
COPY server.py /opt/server.py
ENTRYPOINT ["python", "/opt/server.py"]
