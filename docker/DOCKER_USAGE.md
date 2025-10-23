# CaptchAPI - Docker Usage Guide

This guide explains how to run CaptchAPI in Docker with different persistence strategies while maintaining security with nonroot containers.

## Quick Start

```bash
# From project root
cd docker

# Build the image
docker build -f Dockerfile -t captchapi:latest ..

# Run with Docker Compose (recommended)
docker-compose up -d
```

---

## Three Persistence Options

CaptchAPI runs as **nonroot user (UID 65532)** for security. Choose the persistence option that fits your needs:

### **Option 1: Transient (No Persistence)** 🚀

Database lives in the container and is **lost on restart**. Perfect for testing or stateless deployments.

**Pros:**
- ✅ Works immediately, zero configuration
- ✅ No volume permission issues
- ✅ Fully secure (nonroot)

**Cons:**
- ❌ Data lost when container stops

**Usage:**
```bash
# Docker CLI
docker run -d -p 3000:3000 \
  -e API_KEY_SALT=your-salt \
  -e MASTER_API_KEY=your-master-key \
  captchapi:latest

# Docker Compose - uncomment 'captchapi-transient' in docker-compose.yml
```

---

### **Option 2: Docker Volume (Recommended)** ⭐

Docker-managed volume with automatic permission handling via init container.

**Pros:**
- ✅ Persistent across restarts
- ✅ Automatic permission setup
- ✅ Fully secure (nonroot)
- ✅ Easy backups
- ✅ Works across Docker hosts

**Cons:**
- None!

**Usage:**

#### **With Docker Compose (Easiest):**
```bash
# The default docker-compose.yml already includes the init container
docker-compose up -d

# View logs
docker-compose logs -f captchapi

# Stop
docker-compose down

# Stop and remove data
docker-compose down -v
```

#### **With Docker CLI:**
```bash
# 1. Create volume
docker volume create captchapi-data

# 2. Fix permissions (one-time)
docker run --rm -v captchapi-data:/data alpine chown -R 65532:65532 /data

# 3. Run container
docker run -d -p 3000:3000 \
  -v captchapi-data:/data \
  -e API_KEY_SALT=your-salt \
  -e MASTER_API_KEY=your-master-key \
  captchapi:latest
```

---

### **Option 3: Bind Mount (Host Directory)** 📁

Mount a host directory into the container. Useful when you need direct access to the database file.

**Pros:**
- ✅ Persistent across restarts
- ✅ Direct access to files on host
- ✅ Easy to backup/copy
- ✅ Fully secure (nonroot)

**Cons:**
- ⚠️ Requires one-time permission setup on host

**Usage:**

#### **Step 1: Create and prepare host directory**
```bash
# Create directory
mkdir -p ./data

# Set ownership to UID 65532 (nonroot user in container)
sudo chown -R 65532:65532 ./data

# OR if you don't have sudo, use your UID and run container as your user
chown -R $(id -u):$(id -g) ./data
# Then add: -u $(id -u):$(id -g) to docker run command
```

#### **Step 2: Run container**

**With Docker Compose:**
```bash
# Edit docker-compose.yml: uncomment the 'captchapi-bindmount' service
docker-compose up captchapi-bindmount -d
```

**With Docker CLI:**
```bash
docker run -d -p 3000:3000 \
  -v $(pwd)/data:/data \
  -e API_KEY_SALT=your-salt \
  -e MASTER_API_KEY=your-master-key \
  captchapi:latest
```

---

## Environment Variables

All environment variables with defaults:

```bash
# Server
SERVER_HOST=0.0.0.0          # Must be 0.0.0.0 in container
SERVER_PORT=3000

# Database
DATABASE_URL=sqlite:/data/captchapi.db

# Security (CHANGE THESE IN PRODUCTION!)
API_KEY_SALT=change-this-to-a-random-salt-in-production
MASTER_API_KEY=change-this-to-a-secure-master-key-in-production

# CAPTCHA
DEFAULT_SESSION_TTL_SECONDS=300
MAX_SESSION_TTL_SECONDS=3600
MAX_VALIDATION_ATTEMPTS=3

# Background Tasks
CLEANUP_INTERVAL_SECONDS=60
```

**Generate secure keys:**
```bash
# Generate random salt
openssl rand -base64 32

# Generate random master key
openssl rand -base64 32
```

---

## Complete Examples

### **Production Setup (Docker Volume)**

```bash
# 1. Create .env file
cat > .env << EOF
API_KEY_SALT=$(openssl rand -base64 32)
MASTER_API_KEY=$(openssl rand -base64 32)
EOF

# 2. Create docker-compose.prod.yml
cat > docker-compose.prod.yml << 'EOF'
services:
  captchapi:
    image: captchapi:latest
    container_name: captchapi-prod
    restart: always
    ports:
      - "3000:3000"
    volumes:
      - captchapi-data:/data
    env_file:
      - .env
    depends_on:
      init-perms:
        condition: service_completed_successfully

  init-perms:
    image: alpine:latest
    volumes:
      - captchapi-data:/data
    command: chown -R 65532:65532 /data
    restart: "no"

volumes:
  captchapi-data:
EOF

# 3. Start
docker-compose -f docker-compose.prod.yml up -d

# 4. Check health
curl http://localhost:3000/health
```

### **Development Setup (Transient)**

```bash
# Quick start for development
docker run -d \
  --name captchapi-dev \
  -p 3000:3000 \
  -e API_KEY_SALT=dev-salt \
  -e MASTER_API_KEY=dev-master-key \
  captchapi:latest

# Create an API key
curl -X POST http://localhost:3000/api/v1/api-keys \
  -H "Authorization: Bearer dev-master-key" \
  -H "Content-Type: application/json" \
  -d '{"description": "Dev Key"}'
```

---

## Troubleshooting

### **Permission Denied Errors**

```bash
# Error: "unable to open database file"
# Solution: Fix volume permissions

# For Docker volumes:
docker run --rm -v captchapi_captchapi-data:/data alpine chown -R 65532:65532 /data

# For bind mounts:
sudo chown -R 65532:65532 ./data
```

### **Check Running Container**

```bash
# View logs
docker logs captchapi

# Check if running as nonroot
docker exec captchapi whoami
# Should output: nonroot

# Check file permissions
docker exec captchapi ls -la /data
```

### **Database Backup**

```bash
# Backup Docker volume
docker run --rm \
  -v captchapi_captchapi-data:/data \
  -v $(pwd):/backup \
  alpine tar czf /backup/captchapi-backup-$(date +%Y%m%d).tar.gz /data

# Restore
docker run --rm \
  -v captchapi_captchapi-data:/data \
  -v $(pwd):/backup \
  alpine tar xzf /backup/captchapi-backup-YYYYMMDD.tar.gz -C /
```

---

## Security Notes

### **Why Nonroot?**

The container runs as UID 65532 (nonroot) following security best practices:
- Prevents privilege escalation if container is compromised
- Limits damage from container escape vulnerabilities
- Required for many compliance standards (PCI-DSS, SOC2, etc.)

### **Production Checklist**

- [ ] Change `API_KEY_SALT` to random value
- [ ] Change `MASTER_API_KEY` to random value
- [ ] Use Docker volumes (not bind mounts) in production
- [ ] Enable HTTPS via reverse proxy (nginx/traefik)
- [ ] Implement rate limiting at proxy level
- [ ] Regular database backups
- [ ] Monitor logs for suspicious activity
- [ ] Keep Docker image updated

---

## Advanced: Kubernetes Deployment

```yaml
apiVersion: v1
kind: PersistentVolumeClaim
metadata:
  name: captchapi-data
spec:
  accessModes:
    - ReadWriteOnce
  resources:
    requests:
      storage: 1Gi
---
apiVersion: apps/v1
kind: Deployment
metadata:
  name: captchapi
spec:
  replicas: 1
  selector:
    matchLabels:
      app: captchapi
  template:
    metadata:
      labels:
        app: captchapi
    spec:
      # Init container to fix permissions
      initContainers:
      - name: fix-perms
        image: alpine:latest
        command: ["chown", "-R", "65532:65532", "/data"]
        volumeMounts:
        - name: data
          mountPath: /data
      # Main application
      containers:
      - name: captchapi
        image: captchapi:latest
        ports:
        - containerPort: 3000
        env:
        - name: API_KEY_SALT
          valueFrom:
            secretKeyRef:
              name: captchapi-secrets
              key: api-key-salt
        - name: MASTER_API_KEY
          valueFrom:
            secretKeyRef:
              name: captchapi-secrets
              key: master-api-key
        volumeMounts:
        - name: data
          mountPath: /data
        securityContext:
          runAsUser: 65532
          runAsGroup: 65532
          allowPrivilegeEscalation: false
          readOnlyRootFilesystem: false
      volumes:
      - name: data
        persistentVolumeClaim:
          claimName: captchapi-data
```

---

## Support

For issues or questions:
- Check logs: `docker logs captchapi`
- Review API documentation: `.claude/docs/API.md`
- Testing guide: `.claude/docs/TESTING.md`

**Common Issues:**
- Port 3000 already in use → Change host port: `-p 8080:3000`
- Permission denied → Fix volume permissions (see Troubleshooting above)
- Container exits immediately → Check logs for configuration errors
